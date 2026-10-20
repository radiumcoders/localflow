//! `localflow --selftest`: each piece that doesn't need a person, one line
//! per check (PASS / SKIP / FAIL).

use std::process::ExitCode;
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use localflow_core::{Config, Localflow};
use localflow_desktop::hotkey::{self, HotkeyEvent};
use localflow_desktop::paste::{clipboard_get, clipboard_set};
use localflow_desktop::target::{Target, focused_window, foreground};
use localflow_speech::{RATE, Recorder, level, models, read_wav, wav_bytes};

use crate::ui::{self, Start, UiMsg};

enum Outcome {
    Pass(String),
    Skip(String),
}

type Check = Result<Outcome, String>;

fn pass(detail: impl Into<String>) -> Check {
    Ok(Outcome::Pass(detail.into()))
}

fn skip(why: impl Into<String>) -> Check {
    Ok(Outcome::Skip(why.into()))
}

pub fn run(seconds: f32) -> ExitCode {
    println!("localflow self-test on {} {}", std::env::consts::OS, std::env::consts::ARCH);
    let mut failed = 0;
    let mut check = |name: &str, f: &dyn Fn() -> Check| match f() {
        Ok(Outcome::Pass(d)) => println!("PASS {name}{}", if d.is_empty() { String::new() } else { format!(": {d}") }),
        Ok(Outcome::Skip(why)) => println!("SKIP {name}: {why}"),
        Err(e) => {
            failed += 1;
            println!("FAIL {name}: {e}");
        }
    };
    check("audio and WAV", &audio);
    check("cleanup settings", &|| {
        let cfg = Config::from_env();
        pass(format!("{} via {}", if cfg.model.is_empty() { "no model" } else { &cfg.model }, cfg.backend()))
    });
    check("speech model", &speech_model);
    check("focused app and project", &|| {
        let w = focused_window();
        let t = Target::new(&w);
        pass(format!("{:?} {:?}, project {:?}, {} files (title {} chars, pid {:?})", t.kind, t.app, t.project,
                     t.files.len(), w.title.chars().count(), w.pid))
    });
    check("reading the focused text box", &|| {
        if cfg!(target_os = "linux") {
            return skip("Linux learns from what you send (no text box access)");
        }
        match localflow_desktop::learn::focused_text() {
            Some((pid, text)) => pass(format!("read {} characters from pid {pid}", text.chars().count())),
            None => pass("no text box focused (read without errors)"),
        }
    });
    check("clipboard", &clipboard);
    check("hotkey", &hotkey_check);
    check("microphone", &microphone);
    check("HTTPS", &https);
    check("overlay", &|| overlay(seconds));
    println!("\n{failed} failed");
    if failed > 0 { ExitCode::FAILURE } else { ExitCode::SUCCESS }
}

fn audio() -> Check {
    let tone: Vec<i16> = (0..RATE).map(|i| (8000.0 * (i as f32 / 8.0).sin()) as i16).collect();
    let back = read_wav(&wav_bytes(&tone, RATE)).map_err(|e| e.0)?;
    if back != tone {
        return Err("WAV round trip changed the samples".into());
    }
    let (loud, quiet) = (level(&tone), level(&[0; 3200]));
    if loud <= 0.5 || quiet != 0.0 {
        return Err(format!("levels: tone {loud:.2}, silence {quiet:.2}"));
    }
    pass(format!("tone level {loud:.2}, silence 0.00"))
}

fn speech_model() -> Check {
    let cfg = Config::from_env();
    let model = models::resolve(cfg.speech_model.as_deref());
    if !models::installed(model.id) {
        return skip(format!("{} isn't downloaded (localflow models download {})", model.id, model.id));
    }
    // A second of near-silence: loads the model and runs it end to end.
    let lf = Localflow::new(cfg);
    let start = Instant::now();
    let heard = crate::dictation::transcribe(&lf, &vec![0; RATE as usize])?;
    pass(format!("{} loaded and ran in {} ms (heard {:?})", model.id, start.elapsed().as_millis(), heard.text))
}

fn clipboard() -> Check {
    if cfg!(target_os = "linux") && std::env::var_os("WAYLAND_DISPLAY").is_none() && std::env::var_os("DISPLAY").is_none() {
        return skip("no display");
    }
    let sample = "localflow ✓ test\nline two";
    let old = clipboard_get();
    if !clipboard_set(sample) {
        return Err("couldn't set the clipboard".into());
    }
    std::thread::sleep(Duration::from_millis(100));
    let got = clipboard_get().map(|t| t.replace("\r\n", "\n"));
    if let Some(old) = old {
        clipboard_set(&old);
    }
    if got.as_deref() != Some(sample) {
        return Err(format!("read back {got:?}"));
    }
    pass("set and read back unicode + newlines")
}

fn hotkey_check() -> Check {
    if cfg!(target_os = "linux") {
        return skip("Linux uses compositor key bindings");
    }
    let (tx, rx) = mpsc::channel();
    let errors = tx.clone();
    std::thread::spawn(move || {
        if let Err(e) = hotkey::listen(tx, Arc::default()) {
            let _ = errors.send(HotkeyEvent::Error(e));
        }
    });
    match rx.recv_timeout(Duration::from_secs(2)) {
        Ok(HotkeyEvent::Ready) => pass("keyboard hook installed"),
        Ok(HotkeyEvent::Error(e)) if e.contains("Accessibility") => skip(e),
        Ok(HotkeyEvent::Error(e)) => Err(e),
        Ok(other) => Err(format!("first event {other:?}")),
        Err(_) => Err("no answer from the keyboard hook".into()),
    }
}

fn microphone() -> Check {
    let rec = match Recorder::start(|_| {}) {
        Ok(rec) => rec,
        Err(e) => return skip(format!("no microphone ({e})")),
    };
    std::thread::sleep(Duration::from_millis(600));
    let samples = rec.stop();
    if samples.is_empty() {
        return skip("the recorder gave no audio (no microphone?)");
    }
    pass(format!("{:.2}s recorded, level {:.2}", samples.len() as f32 / RATE as f32, level(&samples)))
}

fn https() -> Check {
    let agent = ureq::AgentBuilder::new().timeout(Duration::from_secs(10)).build();
    match agent.get("https://api.github.com").call() {
        // Any answer at all means the certificate checked out.
        Ok(_) | Err(ureq::Error::Status(..)) => pass("certificates verified (api.github.com)"),
        Err(e) if e.to_string().to_lowercase().contains("certificate") => Err(format!("HTTPS certificates can't be verified: {e}")),
        Err(e) => skip(format!("offline? ({e})")),
    }
}

/// Show the pill through recording, working and a message, and check the
/// focused window stays the same.
fn overlay(seconds: f32) -> Check {
    if cfg!(target_os = "linux") && std::env::var_os("WAYLAND_DISPLAY").is_none() && std::env::var_os("DISPLAY").is_none() {
        return skip("no display");
    }
    let before = foreground();
    let samples: Arc<Mutex<Vec<Option<String>>>> = Arc::default();
    let seen = samples.clone();
    let (ui, rx) = ui::channel();
    ui::run(ui, rx, Start {
        settings: false,
        quit_with_settings: false,
        then: Box::new(move |ui, _| {
            std::thread::spawn(move || {
                let start = Instant::now();
                ui.send(UiMsg::Recording { raw: false });
                let mut i = 0;
                while start.elapsed().as_secs_f32() < seconds {
                    i += 1;
                    ui.level((i as f32 / 3.0).sin().abs());
                    match i {
                        30 => ui.send(UiMsg::Working("Transcribing".into())),
                        40 => ui.send(UiMsg::Working("Cleaning up".into())),
                        50 => ui.send(UiMsg::Message("Done".into(), 1000)),
                        _ => {}
                    }
                    seen.lock().unwrap().push(foreground());
                    std::thread::sleep(Duration::from_millis(50));
                }
                ui.send(UiMsg::Quit);
            });
        }),
    });
    let samples = samples.lock().unwrap();
    let Some(before) = before else { return pass("shown recording / working / message (focus not checkable here)") };
    let moved = samples.iter().filter(|f| f.as_deref() != Some(before.as_str())).count();
    if samples.last().is_some_and(|f| f.as_deref() != Some(before.as_str())) {
        return Err(format!("the overlay kept focus: before {before}, at the end {:?}", samples.last()));
    }
    let flicker = if moved > 0 { format!(", but it moved for {moved} of {} checks", samples.len()) } else { String::new() };
    pass(format!("shown recording / working / message, focus unchanged{flicker} ({before})"))
}
