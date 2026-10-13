//! localflow: dictation (the Rust port, in progress: no overlay, hotkey or paste yet).
//!
//!     localflow listen 5               # record 5 s, transcribe with Handy, clean up, print
//!     localflow transcribe talk.wav    # a 16-bit WAV through Handy and the cleanup
//!     localflow record 3 out.wav       # just the microphone, as 16 kHz mono
//!     echo "meet at 3 wait at 2" | localflow
//!     localflow --check                # one tiny request: is the model reachable?
//!     localflow --eval                 # run cases.txt
//!     localflow --eval qwen3.5:2b      # ...against another model
//!     localflow --eval qwen3.5:4b off  # ...draft only, no review pass
//!
//! Settings: LOCALFLOW_* variables or the env file install.py writes (see
//! `localflow_core::Config`).

use std::io::{Read, Write};
use std::process::ExitCode;
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{Duration, Instant};

use localflow_core::{Config, Localflow, Msg, NO_FILES, eval};
use localflow_speech::{RATE, Recorder, level, read_wav, transcribe, wav_bytes};

const USAGE: &str = "localflow: dictation

    localflow listen [SECONDS]       # record (default 5 s), transcribe, clean up, print
    localflow transcribe FILE.wav    # a 16-bit WAV through Handy and the cleanup
    localflow record SECONDS FILE.wav
    echo \"meet at 3 wait at 2\" | localflow
    localflow --check                # one tiny request: is the model reachable?
    localflow --eval [MODEL [REVIEW_MODEL|off]]   # run cases.txt";

/// Record for `seconds`, drawing the level on stderr.
fn record(seconds: f32) -> Result<Vec<i16>, String> {
    let loudness = Arc::new(AtomicU32::new(0));
    let shown = loudness.clone();
    let rec = Recorder::start(move |l| shown.store(l.to_bits(), Ordering::Relaxed)).map_err(|e| e.to_string())?;
    let start = Instant::now();
    while start.elapsed().as_secs_f32() < seconds {
        let l = f32::from_bits(loudness.load(Ordering::Relaxed));
        let bar = "#".repeat((l * 30.0) as usize);
        eprint!("\r  listening {:4.1}s [{bar:<30}]", seconds - start.elapsed().as_secs_f32());
        std::thread::sleep(Duration::from_millis(50));
    }
    eprintln!();
    Ok(rec.stop())
}

/// Speech to cleaned text, with the raw transcript and timings on stderr.
fn dictate(lf: &Localflow, samples: &[i16]) -> ExitCode {
    eprintln!("  {:.1}s of audio, level {:.2}", samples.len() as f32 / RATE as f32, level(samples));
    let start = Instant::now();
    let raw = match transcribe(samples, lf.cfg.speech_model.as_deref()) {
        Ok(raw) => raw,
        Err(e) => {
            eprintln!("localflow: {e}");
            return ExitCode::FAILURE;
        }
    };
    eprintln!("  heard ({} ms): {raw}", start.elapsed().as_millis());
    let start = Instant::now();
    let out = match lf.clean_detailed(&raw, NO_FILES) {
        Ok(r) => {
            eprintln!("  cleaned ({} ms, {})", start.elapsed().as_millis(), r.verdict);
            r.text
        }
        Err(e) => {
            eprintln!("localflow: {e}; the raw transcript instead");
            raw
        }
    };
    println!("{out}");
    ExitCode::SUCCESS
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let lf = Localflow::new(Config::from_env());
    match args.first().map(String::as_str) {
        Some("--check") => {
            let model = &lf.cfg.model;
            match lf.chat(model, &[Msg::user("Reply with just: ok")], 32, None) {
                Ok(_) => println!("ok: {model} via {}", lf.cfg.backend()),
                Err(e) => {
                    eprintln!("localflow: {model} via {}: {e}", lf.cfg.backend());
                    return ExitCode::FAILURE;
                }
            }
        }
        Some("--eval") => {
            let model = args.get(1).unwrap_or(&lf.cfg.model).clone();
            let review = args.get(2).unwrap_or(&lf.cfg.review_model).clone();
            if let Err(e) = eval::run(&lf, &model, &review) {
                eprintln!("localflow: {e}");
                return ExitCode::FAILURE;
            }
        }
        Some("listen") => {
            let seconds = args.get(1).and_then(|s| s.parse().ok()).unwrap_or(5.0);
            return match record(seconds) {
                Ok(samples) => dictate(&lf, &samples),
                Err(e) => {
                    eprintln!("localflow: {e}");
                    ExitCode::FAILURE
                }
            };
        }
        Some("transcribe") => {
            let Some(path) = args.get(1) else {
                eprintln!("{USAGE}");
                return ExitCode::from(2);
            };
            return match std::fs::read(path).map_err(|e| e.to_string()).and_then(|b| read_wav(&b).map_err(|e| e.0)) {
                Ok(samples) => dictate(&lf, &samples),
                Err(e) => {
                    eprintln!("localflow: {path}: {e}");
                    ExitCode::FAILURE
                }
            };
        }
        Some("record") => {
            let (Some(seconds), Some(path)) = (args.get(1).and_then(|s| s.parse().ok()), args.get(2)) else {
                eprintln!("{USAGE}");
                return ExitCode::from(2);
            };
            match record(seconds).and_then(|s| std::fs::write(path, wav_bytes(&s, RATE)).map(|_| s).map_err(|e| e.to_string())) {
                Ok(samples) => eprintln!("  {path}: {:.1}s, level {:.2}", samples.len() as f32 / RATE as f32, level(&samples)),
                Err(e) => {
                    eprintln!("localflow: {e}");
                    return ExitCode::FAILURE;
                }
            }
        }
        Some("-h" | "--help") => println!("{USAGE}"),
        Some(other) => {
            eprintln!("localflow: unknown option {other}\n\n{USAGE}");
            return ExitCode::from(2);
        }
        None => {
            let mut text = String::new();
            if std::io::stdin().read_to_string(&mut text).is_err() {
                eprintln!("localflow: stdin isn't text");
                return ExitCode::FAILURE;
            }
            let out = lf.clean(&text, NO_FILES).unwrap_or_else(|e| {
                // Ollama/API down: pass the text through
                eprintln!("localflow: {e}; passing text through unchanged");
                text.trim().to_string()
            });
            print!("{out}");
            let _ = std::io::stdout().flush();
        }
    }
    ExitCode::SUCCESS
}
