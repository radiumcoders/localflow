//! localflow: talk, and get clean text where you type.
//!
//!     localflow                        # settings (Linux), or the background app
//!                                      # (macOS / Windows: the hotkey; settings the first time)
//!     localflow settings               # the settings window
//!     localflow onboarding             # the first-run window, again
//!     localflow setup [--api P] [--model M] [--speech-model ID]   # the same without the window
//!
//! Dictation (Linux: bind keys to these; nothing runs between dictations):
//!     localflow start [raw]            # start recording (raw: paste the transcript as is)
//!     localflow stop                   # transcribe, clean up, paste
//!     localflow toggle [raw]           # start, or stop if recording
//!     localflow cancel                 # stop recording and throw it away
//!     localflow learn                  # learn from the dictation you just fixed
//!
//! Tools:
//!     localflow models [download ID]   # speech models: list, or download one
//!     localflow ollama [MODEL]         # install Ollama if it isn't here, start it, pull the model
//!     localflow listen [SECONDS]       # record, transcribe, clean up, print
//!     localflow transcribe FILE.wav    # a 16-bit WAV through the speech model and the cleanup
//!     localflow record SECONDS FILE.wav
//!     echo "meet at 3 wait at 2" | localflow -
//!     localflow --check                # one tiny request: is the cleanup model reachable?
//!     localflow --eval [MODEL [REVIEW_MODEL|off]]   # run cases.txt
//!     localflow --selftest             # check each piece that doesn't need a person
//!
//! Settings: LOCALFLOW_* variables or the env file the settings window writes
//! (see `localflow_core::Config`).

#![cfg_attr(windows, windows_subsystem = "windows")]

mod cli;
mod dictation;
mod ollama;
mod platform;
mod selftest;
mod session;
mod setup;
mod ui;

use std::process::ExitCode;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let raw = args.iter().skip(1).any(|a| a == "raw");
    if !args.is_empty() {
        platform::attach_console();
    }
    match args.first().map(String::as_str) {
        None => session::app(),
        Some("settings") => session::settings(),
        Some("onboarding") => session::onboarding(),
        Some("setup") => setup::run(&args[1..]),
        Some("--background") => session::background(),
        Some("start") => session::start(raw),
        Some("stop") => session::signal("stop"),
        Some("cancel") => session::signal("cancel"),
        Some("toggle") => session::toggle(raw),
        Some("learn") => session::learn(),
        Some("_session") => session::run_session(raw),
        Some("_watch") => session::run_watch(&args[1..]),
        Some("_message") => session::run_message(&args[1..]),
        Some("--selftest") => selftest::run(args.get(1).and_then(|s| s.parse().ok()).unwrap_or(3.0)),
        Some(_) => cli::run(&args),
    }
}
