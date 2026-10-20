//! Speech to text in this process, with whisper.cpp. The model is loaded for
//! a dictation and dropped after it, so nothing stays in memory between
//! dictations.
//!
//! Silero voice detection (when its model is installed) finds where you
//! spoke: only those parts go to whisper, so silence costs nothing and can't
//! turn into made-up text, and a recording with no speech is "nothing heard".
//! Words come back with their timing, so each phrase gets its loudness and
//! the quiet ones (muttered asides, voices in the background) are marked.

use std::path::Path;
use std::sync::Once;
use std::time::Instant;

use whisper_rs::{
    FullParams, SamplingStrategy, WhisperContext, WhisperContextParameters, WhisperVadContext, WhisperVadContextParams,
    WhisperVadParams,
};

use crate::audio::RATE;
use crate::models::{self, Model};
use crate::transcript::{self, Transcript, Word};
use crate::{Error, Result};

/// How a transcription runs.
#[derive(Debug, Clone, Default)]
pub struct Options {
    /// "en", "de", ...; None detects it (multilingual models). English-only
    /// models always use English.
    pub language: Option<String>,
    /// Names and terms you use ("Claude", "Pine Script"): whisper is primed
    /// with them, so it spells them that way.
    pub vocabulary: Vec<String>,
    /// CPU threads; 0 for a sensible number for this machine.
    pub threads: usize,
}

/// Settings for [`transcribe`]: what LOCALFLOW_SPEECH_* say.
#[derive(Debug, Clone)]
pub struct SpeechConfig {
    /// A model id from [`models::MODELS`], a path to a ggml file, or None for
    /// the default.
    pub model: Option<String>,
    /// Use the GPU when this build has GPU support (Metal on macOS; Vulkan
    /// when built with the `vulkan` feature).
    pub use_gpu: bool,
    pub language: Option<String>,
    pub vocabulary: Vec<String>,
    pub threads: usize,
}

impl Default for SpeechConfig {
    fn default() -> Self {
        SpeechConfig { model: None, use_gpu: true, language: None, vocabulary: Vec::new(), threads: 0 }
    }
}

impl SpeechConfig {
    /// From LOCALFLOW_SPEECH_MODEL (an id or a file), LOCALFLOW_SPEECH_GPU
    /// (on/off, default on) and LOCALFLOW_SPEECH_LANGUAGE (auto, en, de...),
    /// read through `get` (the environment, or the env file too).
    pub fn from_vars(get: &dyn Fn(&str) -> Option<String>) -> SpeechConfig {
        let var = |k: &str| get(k).map(|v| v.trim().to_string()).filter(|v| !v.is_empty());
        SpeechConfig {
            model: var("LOCALFLOW_SPEECH_MODEL"),
            use_gpu: !matches!(var("LOCALFLOW_SPEECH_GPU").as_deref(), Some("0" | "off" | "false" | "no" | "cpu")),
            language: var("LOCALFLOW_SPEECH_LANGUAGE").filter(|l| l != "auto"),
            vocabulary: Vec::new(),
            threads: 0,
        }
    }

    /// The model file this config uses, and its catalog entry (None for a
    /// file named by path).
    pub fn model_file(&self) -> (std::path::PathBuf, Option<&'static Model>) {
        if let Some(m) = self.model.as_deref().filter(|m| m.ends_with(".bin") && Path::new(m).is_file()) {
            return (m.into(), None);
        }
        let model = models::resolve(self.model.as_deref());
        (model.path(), Some(model))
    }

    pub fn options(&self) -> Options {
        Options { language: self.language.clone(), vocabulary: self.vocabulary.clone(), threads: self.threads }
    }
}

/// Load the model, transcribe, and drop the model again.
pub fn transcribe(samples: &[i16], cfg: &SpeechConfig) -> Result<Transcript> {
    let (path, model) = cfg.model_file();
    if !path.is_file() {
        let name = model.map_or_else(|| path.display().to_string(), |m| m.name.to_string());
        return Err(Error(format!("speech model {name} isn't downloaded yet (open localflow's settings)")));
    }
    Engine::load(&path, cfg.use_gpu)?.transcribe(samples, &cfg.options())
}

fn quiet_logs() {
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        // Without a log backend these route whisper.cpp's and ggml's logging nowhere.
        if std::env::var_os("LOCALFLOW_DEBUG").is_none() {
            whisper_rs::install_logging_hooks();
        }
    });
}

fn default_threads() -> usize {
    // Physical cores do the work; hyperthreads don't help whisper.
    std::thread::available_parallelism().map_or(4, |n| (n.get() / 2).clamp(1, 8))
}

/// A loaded speech model (and the voice detector, if installed).
pub struct Engine {
    ctx: WhisperContext,
    vad: Option<WhisperVadContext>,
    english_only: bool,
}

/// A stretch of the recording that went to whisper: where it starts in the
/// audio whisper got, where in the recording, and how long it is (seconds).
#[derive(Debug, Clone, Copy)]
struct Piece {
    at: f32,
    orig: f32,
    len: f32,
}

/// Silence put between the pieces of speech sent to whisper (seconds).
const GAP_S: f32 = 0.2;
/// A pause this long (seconds) starts a new phrase.
const PHRASE_GAP_S: f32 = 0.5;

impl Engine {
    /// Load a ggml model file. With `use_gpu`, whisper runs on the GPU when
    /// this build supports one (and falls back to the CPU otherwise).
    pub fn load(path: &Path, use_gpu: bool) -> Result<Engine> {
        quiet_logs();
        let path_str = path.to_str().ok_or_else(|| Error(format!("{}: not a UTF-8 path", path.display())))?;
        let mut params = WhisperContextParameters::default();
        params.use_gpu(use_gpu).flash_attn(std::env::var_os("LOCALFLOW_NOFA").is_none());
        let ctx = WhisperContext::new_with_params(path_str, params)
            .map_err(|e| Error(format!("couldn't load the speech model {}: {e}", path.display())))?;
        let english_only = !ctx.is_multilingual();
        let vad = models::VAD.installed().then(|| models::VAD.path()).and_then(|p| {
            let mut vp = WhisperVadContextParams::default();
            vp.set_n_threads(2);
            vp.set_use_gpu(false); // tiny; the GPU would only add setup time
            WhisperVadContext::new(p.to_str()?, vp).ok()
        });
        Ok(Engine { ctx, vad, english_only })
    }

    pub fn has_vad(&self) -> bool {
        self.vad.is_some()
    }

    /// Transcribe the whole recording, silence included (for comparisons).
    pub fn disable_vad(&mut self) {
        self.vad = None;
    }

    /// Where there's speech in the recording, as (start, end) seconds; None
    /// without the voice detector.
    fn speech_spans(&mut self, audio: &[f32]) -> Option<Vec<(f32, f32)>> {
        let vad = self.vad.as_mut()?;
        let mut p = WhisperVadParams::default();
        p.set_threshold(0.5);
        p.set_min_speech_duration(200);
        p.set_min_silence_duration(300);
        p.set_speech_pad(200);
        let segments = vad.segments_from_samples(p, audio).ok()?;
        let total = audio.len() as f32 / RATE as f32;
        let mut spans: Vec<(f32, f32)> = Vec::new();
        for s in segments {
            let (a, b) = ((s.start / 100.0).max(0.0), (s.end / 100.0).min(total)); // centiseconds
            match spans.last_mut() {
                Some(last) if a <= last.1 + 0.1 => last.1 = last.1.max(b),
                _ if b > a => spans.push((a, b)),
                _ => {}
            }
        }
        Some(spans)
    }

    /// Speech (16 kHz mono) to text. An empty transcript means nothing was said.
    pub fn transcribe(&mut self, samples: &[i16], opts: &Options) -> Result<Transcript> {
        let debug = std::env::var_os("LOCALFLOW_DEBUG").is_some();
        let started = Instant::now();
        let audio: Vec<f32> = samples.iter().map(|&s| f32::from(s) / 32768.0).collect();
        let total = audio.len() as f32 / RATE as f32;
        if total < 0.3 {
            return Ok(Transcript::default());
        }
        let spans = self.speech_spans(&audio);
        if debug {
            eprintln!("  vad: {spans:?} ({} ms)", started.elapsed().as_millis());
        }
        let spans = match spans {
            Some(s) if s.is_empty() => return Ok(Transcript::default()), // no speech at all
            Some(s) => s,
            None => vec![(0.0, total)],
        };
        let speech_s: f32 = spans.iter().map(|(a, b)| b - a).sum();
        // The speech, back to back, a short silence between pieces.
        let mut input = Vec::with_capacity(audio.len());
        let mut pieces = Vec::new();
        for &(a, b) in &spans {
            let (i, j) = ((a * RATE as f32) as usize, ((b * RATE as f32) as usize).min(audio.len()));
            if !input.is_empty() {
                input.resize(input.len() + (GAP_S * RATE as f32) as usize, 0.0);
            }
            pieces.push(Piece { at: input.len() as f32 / RATE as f32, orig: a, len: (j - i) as f32 / RATE as f32 });
            input.extend_from_slice(&audio[i..j]);
        }
        // whisper wants at least a second
        if input.len() < RATE as usize + RATE as usize / 10 {
            input.resize(RATE as usize + RATE as usize / 10, 0.0);
        }

        let mut params = FullParams::new(SamplingStrategy::Greedy { best_of: 1 });
        let threads = if opts.threads == 0 { default_threads() } else { opts.threads };
        params.set_n_threads(threads as i32);
        let language = if self.english_only {
            Some("en".to_string())
        } else {
            opts.language.as_deref().map(|l| l.replace('\0', "")).filter(|l| !l.is_empty() && l != "auto")
        };
        params.set_language(Some(language.as_deref().unwrap_or("auto")));
        params.set_translate(false);
        params.set_no_context(true);
        params.set_suppress_blank(true);
        params.set_suppress_nst(true);
        params.set_temperature(0.0);
        params.set_print_special(false);
        params.set_print_progress(false);
        params.set_print_realtime(false);
        params.set_print_timestamps(false);
        // A segment per word, with its timing
        params.set_token_timestamps(true);
        params.set_max_len(1);
        params.set_split_on_word(true);
        // whisper encodes 30 s at a time, silence included: for a short
        // dictation, encode just its length (50 frames a second, plus some
        // room), several times faster.
        let input_s = input.len() as f32 / RATE as f32;
        if input_s < 28.0 && std::env::var_os("LOCALFLOW_SPEECH_FULL_WINDOW").is_none() {
            params.set_audio_ctx((((input_s + 1.0) * 50.0) as u32).div_ceil(64) as i32 * 64);
        }
        let prompt = vocabulary_prompt(&opts.vocabulary);
        if !prompt.is_empty() {
            params.set_initial_prompt(&prompt);
        }

        let mut state = self.ctx.create_state().map_err(|e| Error(format!("speech engine: {e}")))?;
        state.full(params, &input).map_err(|e| Error(format!("couldn't transcribe: {e}")))?;
        // whisper's time (in `input`) to the recording's, and which piece it's in
        let piece = |t: f32| pieces.iter().rposition(|p| p.at <= t + 1e-3).unwrap_or(0);
        let to_orig = |t: f32| {
            let p = &pieces[piece(t)];
            p.orig + (t - p.at).clamp(0.0, p.len)
        };
        let words: Vec<Word> = state
            .as_iter()
            .filter_map(|s| {
                let text = s.to_str_lossy().ok()?.into_owned();
                let (a, b) = (s.start_timestamp() as f32 / 100.0, s.end_timestamp() as f32 / 100.0);
                // whisper starts a word right where the one before ended,
                // even across a pause: its end says which piece it's in
                let b = b.max(a);
                let span = piece((b - 0.02).max(a));
                let start = if piece(a) == span { to_orig(a) } else { pieces[span].orig };
                Some(Word { text: transcript::strip_noises(&text), start_s: start, end_s: to_orig(b), span })
            })
            .collect();
        drop(state);
        let mut segments = transcript::phrases(&words, samples, PHRASE_GAP_S, &spans);
        let mut out = Transcript::from_segments(segments.clone());
        let loudness = spans
            .iter()
            .map(|&(a, b)| transcript::db_to_level(transcript::speech_db(samples, a, b)))
            .fold(0.0, f32::max);
        if transcript::is_phantom(&out.text, speech_s, loudness) {
            if debug {
                eprintln!("  dropped as made up: {:?}", out.text);
            }
            return Ok(Transcript::default());
        }
        transcript::mark_quiet(&mut segments);
        if debug {
            for p in &segments {
                eprintln!(
                    "  phrase {:6.2}-{:6.2} {:6.1} dB{} {}",
                    p.start_s,
                    p.end_s,
                    p.db,
                    if p.quiet { " quiet" } else { "" },
                    p.text
                );
            }
        }
        out.segments = transcript::merge_runs(segments);
        if debug {
            eprintln!("  transcribed {total:.1}s ({speech_s:.1}s speech) in {} ms", started.elapsed().as_millis());
        }
        Ok(out)
    }
}

/// whisper's initial prompt from your vocabulary: "Claude, Pine Script." It
/// reads it as text that came before, so it spells those names that way. Kept
/// short: whisper only looks at the last ~220 tokens of it.
fn vocabulary_prompt(words: &[String]) -> String {
    let mut out = String::new();
    for w in words {
        let w = w.replace('\0', "");
        let w = w.trim();
        if w.is_empty() || out.split(", ").any(|x| x == w) {
            continue;
        }
        if out.len() + w.len() + 2 > 600 {
            break;
        }
        if !out.is_empty() {
            out.push_str(", ");
        }
        out.push_str(w);
    }
    if !out.is_empty() {
        out.push('.');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prompt_from_vocabulary() {
        let v = |w: &[&str]| w.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert_eq!(vocabulary_prompt(&v(&["Claude", " Pine Script ", "", "Claude"])), "Claude, Pine Script.");
        assert_eq!(vocabulary_prompt(&[]), "");
        let long: Vec<String> = (0..500).map(|i| format!("Term{i}")).collect();
        assert!(vocabulary_prompt(&long).len() <= 601);
    }

    #[test]
    fn config_from_vars() {
        let vars = |pairs: &'static [(&'static str, &'static str)]| {
            move |k: &str| pairs.iter().find(|(n, _)| *n == k).map(|(_, v)| v.to_string())
        };
        let c = SpeechConfig::from_vars(&vars(&[]));
        assert!(c.model.is_none() && c.use_gpu && c.language.is_none());
        assert_eq!(c.model_file().1.unwrap().id, models::DEFAULT);
        let c = SpeechConfig::from_vars(&vars(&[
            ("LOCALFLOW_SPEECH_MODEL", "base.en"),
            ("LOCALFLOW_SPEECH_GPU", "off"),
            ("LOCALFLOW_SPEECH_LANGUAGE", "auto"),
        ]));
        assert_eq!(c.model.as_deref(), Some("base.en"));
        assert!(!c.use_gpu && c.language.is_none());
        assert_eq!(c.model_file().1.unwrap().id, "base.en");
        let c = SpeechConfig::from_vars(&vars(&[("LOCALFLOW_SPEECH_LANGUAGE", "de")]));
        assert_eq!(c.language.as_deref(), Some("de"));
    }

    #[test]
    fn engine_moves_between_threads() {
        // Loaded on a worker while you speak, used after.
        fn send<T: Send>() {}
        send::<Engine>();
    }

    #[test]
    fn missing_model_says_so() {
        let cfg = SpeechConfig { model: Some("tiny.en".into()), ..Default::default() };
        if !models::installed("tiny.en") {
            let e = transcribe(&[0; 16000], &cfg).unwrap_err();
            assert!(e.0.contains("isn't downloaded"), "{e}");
        }
    }
}
