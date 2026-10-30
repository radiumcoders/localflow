//! One dictation: record, transcribe, clean up, paste, and keep what was
//! pasted for the learn key. Runs on worker threads; the overlay hears about
//! it through `Ui`.

use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use localflow_core::agent::{Command, Submit, command, split_send};
use localflow_core::snippets::snippet;
use localflow_core::{Config, Localflow};
use localflow_desktop::history::{self, Entry};
use localflow_desktop::learn::remember;
use localflow_desktop::paste::{Key, paste_text, press};
use localflow_desktop::paths::{info, warn};
use localflow_desktop::project::file_candidates;
use localflow_desktop::target::{Target, deliver, focused_window};
use localflow_speech::{RATE, Recorder, Transcript, level, read_wav};

use crate::ui::{Ui, UiMsg};

/// Speech to text with the model from the settings, nudged
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
                // Load the local model while you speak (nothing to do on an API).
                if let Err(e) = warm.warm() {
                    info(format_args!("warm failed: {e}"));
                }
            });
            let found = target.clone();
            finder = Some(std::thread::spawn(move || {
                let t = Target::new(&focused_window());
                info(format_args!("dictating into {t}"));
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
        // In a coding agent: a command presses keys; "send it" sends the prompt.
        let agent = target.as_ref().and_then(|t| t.agent.clone());
        let keep = |text: &str, outcome: &str| {
            if self.lf.cfg.history {
                let app = target.as_ref().map(|t| t.app.clone()).unwrap_or_default();
                let (heard, text, outcome) = (heard.text.clone(), text.to_string(), outcome.to_string());
                history::add(Entry { app, agent: agent.clone(), heard, text, outcome, ..Entry::default() });
            }
        };
        // A snippet's trigger, and nothing else: its text as written, without
        // the cleanup. Only pasted, even in a coding agent. Not kept for the
        // learn key: a fix to it is a new snippet, not a misheard word.
        let snippets = if self.raw { vec![] } else { self.lf.snippets() };
        if let Some(snippet) = snippet(&heard.text, &snippets) {
            self.ui.send(UiMsg::Hide);
            std::thread::sleep(Duration::from_millis(100)); // the overlay goes before the paste
            let pasted = deliver(&snippet.text, target.as_ref(), &[]);
            keep(&snippet.text, if pasted { "snippet" } else { "copied" });
            info(format_args!("{secs:.1}s of audio: a snippet{}", if pasted { "" } else { " (left on the clipboard)" }));
            if !pasted {
                return message(&copied(), 2500);
            }
            return message("Pasted a snippet", 1200);
        }
        if let Some(agent) = &agent
            && let Some(cmd) = command(&heard.text, agent)
        {
            self.ui.send(UiMsg::Hide);
            std::thread::sleep(Duration::from_millis(100));
            let done = run_command(cmd);
            keep(cmd.describe(), "command");
            info(format_args!("{secs:.1}s of audio: {} to {agent}{}", cmd.describe(), if done { "" } else { " (keys not sent)" }));
            return message(if done { cmd.describe() } else { "Couldn't press the keys" }, 1200);
        }
        let mut input = heard.for_cleanup(true);
        let mut send = agent.is_some() && self.lf.cfg.submit == Submit::Always;
        let mut as_heard = false;
        if agent.is_some() && self.lf.cfg.submit == Submit::Said {
            let (rest, said_send) = split_send(&input);
            (input, send) = (rest, said_send);
        }
        let text = if self.raw {
            heard.text.clone()
        } else {
            self.ui.send(UiMsg::Working("Cleaning up".into()));
            offered = target.as_ref().map(|t| file_candidates(&heard.text, &t.files, 12)).unwrap_or_default();
            match self.lf.clean(&input, &offered) {
                Ok(text) => text,
                Err(e) => {
                    // Never lose a dictation: the transcript as heard.
                    warn(format_args!("cleanup failed, pasting the raw transcript: {e}"));
                    as_heard = true;
                    input.replace("<quiet>", "").replace("</quiet>", "")
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
        let sent = pasted && send && {
            std::thread::sleep(Duration::from_millis(150)); // the agent takes the paste first
            press(Key::Enter)
        };
        keep(&text, if sent { "sent" } else if pasted { "pasted" } else { "copied" });
        remember(&text);
        info(format_args!(
            "{secs:.1}s of audio: transcribed in {:.2}s, done in {:.2}s{}{}{}",
            t_stt.as_secs_f32(),
            start.elapsed().as_secs_f32(),
            if offered.is_empty() { String::new() } else { format!(" ({} files offered)", offered.len()) },
            if quiet > 0 { format!(" ({quiet} quiet segments)") } else { String::new() },
            if sent { " and sent" } else if pasted { "" } else { " (left on the clipboard)" },
        ));
        if !pasted {
            return message(&copied(), 2500);
        }
        // A moment of "it went in", once the text is there (and a word when
        // the cleanup couldn't run: the transcript as heard went in).
        if as_heard {
            return message("Pasted as heard (no cleanup)", 1800);
        }
        message(if sent { "Sent" } else { "Pasted" }, 900)
    }
}

/// Press a command's keys in the focused window.
fn run_command(cmd: Command) -> bool {
    match cmd {
        Command::Stop => press(Key::Escape),
        Command::Rewind => {
            press(Key::Escape) && {
                std::thread::sleep(Duration::from_millis(80));
                press(Key::Escape)
            }
        }
        Command::Slash(slash) => {
            paste_text(slash) && {
                std::thread::sleep(Duration::from_millis(150));
                press(Key::Enter)
            }
        }
        Command::Send => press(Key::Enter),
    }
}

/// For the pill when the paste keys couldn't be sent.
fn copied() -> String {
    let keys = if cfg!(target_os = "macos") { "Cmd+V" } else { "Ctrl+V" };
    format!("Copied: paste with {keys}")
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
