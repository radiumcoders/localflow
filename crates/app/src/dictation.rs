//! One dictation: record, transcribe, clean up, paste, then watch for your
//! fixes. Runs on worker threads; the overlay hears about it through `Ui`.

use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use localflow_core::{Config, Localflow};
use localflow_desktop::learn::{Learning, learning_mode, remember};
use localflow_desktop::paths::{info, warn};
use localflow_desktop::project::file_candidates;
use localflow_desktop::target::{Target, deliver, focused_window};
use localflow_speech::{RATE, Recorder, Transcript, level, read_wav};

use crate::ui::{Ui, UiMsg};

/// Speech to text with the model and language from the settings, nudged
/// towards the spellings in your vocabulary.
pub fn transcribe(lf: &Localflow, samples: &[i16]) -> Result<Transcript, String> {
    let words: Vec<String> = lf
        .vocabulary()
        .iter()
        .map(|w| w.split(':').next().unwrap_or(w).trim().to_string())
        .filter(|w| !w.is_empty())
        .collect();
    localflow_speech::transcribe(samples, &speech_config(&lf.cfg, words)).map_err(|e| e.to_string())
}

fn speech_config(cfg: &Config, vocabulary: Vec<String>) -> localflow_speech::SpeechConfig {
    localflow_speech::SpeechConfig {
        model: cfg.speech_model.clone(),
        use_gpu: cfg.speech_gpu,
        language: cfg.language.clone(),
        vocabulary,
        ..Default::default()
    }
}

/// The microphone, or for tests (LOCALFLOW_TEST_WAV) a WAV file played into
/// the dictation in real time, so CI machines without one dictate end to end.
enum Mic {
    Real(Recorder),
    File { samples: Arc<Mutex<Vec<i16>>>, stop: Arc<std::sync::atomic::AtomicBool> },
}

impl Mic {
    fn start(ui: &Ui) -> Result<Mic, String> {
        let Some(path) = std::env::var_os("LOCALFLOW_TEST_WAV") else {
            let level = ui.clone();
            return Recorder::start(move |l| level.level(l)).map(Mic::Real).map_err(|e| e.to_string());
        };
        let wav = std::fs::read(&path).map_err(|e| e.to_string()).and_then(|b| read_wav(&b).map_err(|e| e.0))?;
        let (samples, stop): (Arc<Mutex<Vec<i16>>>, Arc<std::sync::atomic::AtomicBool>) = Default::default();
        let (into, stopped, ui) = (samples.clone(), stop.clone(), ui.clone());
        std::thread::spawn(move || {
            for chunk in wav.chunks(RATE as usize / 10) {
                if stopped.load(std::sync::atomic::Ordering::Relaxed) {
                    break;
                }
                ui.level(level(chunk));
                into.lock().unwrap().extend_from_slice(chunk);
                std::thread::sleep(Duration::from_millis(100));
            }
        });
        Ok(Mic::File { samples, stop })
    }

    fn stop(self) -> Vec<i16> {
        match self {
            Mic::Real(recorder) => recorder.stop(),
            Mic::File { samples, stop } => {
                stop.store(true, std::sync::atomic::Ordering::Relaxed);
                std::mem::take(&mut *samples.lock().unwrap())
            }
        }
    }
}

pub struct Dictation {
    ui: Ui,
    raw: bool,
    mic: Mic,
    /// Where this goes (the app, its project's files), found while you talk.
    target: Arc<Mutex<Option<Target>>>,
    finder: Option<JoinHandle<()>>,
    lf: Arc<Localflow>,
}

impl Dictation {
    /// Start recording now; the overlay and the rest catch up.
    pub fn start(ui: Ui, raw: bool) -> Result<Dictation, String> {
        let mic = Mic::start(&ui)?;
        ui.send(UiMsg::Recording { raw });
        let lf = Arc::new(Localflow::new(Config::from_env()));
        let target: Arc<Mutex<Option<Target>>> = Arc::default();
        let mut finder = None;
        if !raw {
            let warm = lf.clone();
            std::thread::spawn(move || {
                // With Ollama, load the model while you speak (nothing to do on an API).
                if let Err(e) = warm.warm(&warm.cfg.model) {
                    info(format_args!("warm failed: {e}"));
                }
            });
            let found = target.clone();
            finder = Some(std::thread::spawn(move || {
                let t = Target::new(&focused_window());
                info(format_args!("dictating into {:?} {:?}, project {:?}, {} files", t.kind, t.app, t.project, t.files.len()));
                *found.lock().unwrap() = Some(t);
            }));
        }
        Ok(Dictation { ui, raw, mic, target, finder, lf })
    }

    pub fn cancel(self) {
        drop(self.mic.stop());
        self.ui.send(UiMsg::Hide);
        info("cancelled");
    }

    /// Blocking: run it on a worker thread. Returns how long (ms) the closing
    /// message shows, 0 if none.
    pub fn finish(mut self) -> u64 {
        let start = Instant::now();
        let samples = self.mic.stop();
        let secs = samples.len() as f32 / RATE as f32;
        let message = |text: &str, ms: u64| {
            self.ui.send(UiMsg::Message(text.into(), ms));
            ms
        };
        if secs < 0.3 {
            return message("Too short", 1600);
        }
        self.ui.send(UiMsg::Working("Transcribing".into()));
        let heard = match transcribe(&self.lf, &samples) {
            Ok(heard) => heard,
            Err(e) => {
                warn(format_args!("transcription failed: {e}"));
                return message(&short(&e), 3000);
            }
        };
        let t_stt = start.elapsed();
        if heard.text.trim().is_empty() {
            info(format_args!("{secs:.1}s of audio, nothing heard"));
            return message("Nothing heard", 1600);
        }
        let quiet = heard.segments.iter().filter(|s| s.quiet).count();
        let mut offered = vec![];
        let target = if self.raw {
            None
        } else {
            if let Some(finder) = self.finder.take() {
                join_for(finder, Duration::from_secs(3));
            }
            self.target.lock().unwrap().take()
        };
        let text = if self.raw {
            heard.text.clone()
        } else {
            self.ui.send(UiMsg::Working("Cleaning up".into()));
            let input = heard.for_cleanup(self.lf.cfg.asides);
            offered = target.as_ref().map(|t| file_candidates(&heard.text, &t.files, 12)).unwrap_or_default();
            match self.lf.clean(&input, &offered) {
                Ok(text) => text,
                Err(e) => {
                    // Never lose a dictation: the transcript as heard.
                    warn(format_args!("cleanup failed, pasting the raw transcript: {e}"));
                    heard.text.clone()
                }
            }
        };
        if text.trim().is_empty() {
            info(format_args!("{secs:.1}s of audio, only an aside"));
            return message("Left out an aside", 1600);
        }
        self.ui.send(UiMsg::Hide);
        std::thread::sleep(Duration::from_millis(100)); // the overlay goes before the paste
        let pasted = deliver(&text, target.as_ref(), &offered);
        let stamp = remember(&text);
        if pasted && !self.raw {
            crate::session::watch_for_fixes(target.as_ref().and_then(|t| t.pid), stamp);
        }
        info(format_args!(
            "{secs:.1}s of audio: transcribed in {:.2}s, done in {:.2}s{}{}{}",
            t_stt.as_secs_f32(),
            start.elapsed().as_secs_f32(),
            if offered.is_empty() { String::new() } else { format!(" ({} files offered)", offered.len()) },
            if quiet > 0 { format!(" ({quiet} quiet segments)") } else { String::new() },
            if pasted { "" } else { " (left on the clipboard)" },
        ));
        if !pasted {
            let keys = if cfg!(target_os = "macos") { "Cmd+V" } else { "Ctrl+V" };
            return message(&format!("Copied: paste with {keys}"), 2500);
        }
        0
    }
}

/// An error, short enough for the pill.
fn short(e: &str) -> String {
    let e = e.trim();
    if e.chars().count() <= 40 { e.to_string() } else { format!("{}…", e.chars().take(39).collect::<String>()) }
}

/// Wait for a thread, but not longer than `limit`.
fn join_for(handle: JoinHandle<()>, limit: Duration) {
    let start = Instant::now();
    while !handle.is_finished() && start.elapsed() < limit {
        std::thread::sleep(Duration::from_millis(10));
    }
    if handle.is_finished() {
        let _ = handle.join();
    }
}

/// Learning on its own is on (LOCALFLOW_LEARN=auto).
pub fn learns_on_its_own() -> bool {
    learning_mode() == Learning::Auto
}
