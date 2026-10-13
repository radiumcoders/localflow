//! localflow: dictation cleanup (the Rust port; dictating itself comes next).
//!
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

use localflow_core::{Config, Localflow, Msg, NO_FILES, eval};

const USAGE: &str = "localflow: dictation cleanup

    echo \"meet at 3 wait at 2\" | localflow
    localflow --check                # one tiny request: is the model reachable?
    localflow --eval [MODEL [REVIEW_MODEL|off]]   # run cases.txt";

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
