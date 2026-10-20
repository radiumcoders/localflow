//! localflow's dictation cleanup, ported from the Python `localflow` script.
//!
//! A raw speech-to-text transcript goes in; what the speaker meant to write
//! comes out: self-corrections applied, filler dropped, punctuation, lists and
//! headings, names spelled the way the speaker's vocabulary says. Two
//! checkpoints: a few-shot draft, then automatic checks whose problems (if any)
//! a fresh call repairs. Runs on a local Ollama model or a hosted API with your
//! own key (Anthropic always uses Claude Haiku 5.5).
//!
//! ```no_run
//! let lf = localflow_core::Localflow::new(localflow_core::Config::from_env());
//! println!("{}", lf.clean("let's meet at 3 wait at 4", localflow_core::NO_FILES)?);
//! # Ok::<(), localflow_core::Error>(())
//! ```

mod chat;
mod cleanup;
mod config;
mod ollama;
pub mod difflib;
pub mod eval;
pub mod prompts;
mod vocab;

use std::fmt;
use std::sync::atomic::AtomicBool;
use std::time::Duration;

pub use chat::Msg;
pub use cleanup::{Cleaned, Part, file_refs, find_problems, markdown_headings, plausible, prefix_messages, user_turn};
pub use config::{Config, config_dir, home, save_api_key, save_settings};
pub use vocab::{Hint, corrections, edited_region};

#[derive(Debug, Clone, PartialEq)]
pub struct Error(pub String);

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for Error {}

pub type Result<T> = std::result::Result<T, Error>;

/// For a dictation that names no project files.
pub const NO_FILES: &[&str] = &[];

pub(crate) fn err<T>(msg: impl Into<String>) -> Result<T> {
    Err(Error(msg.into()))
}

/// The cleanup engine: settings, plus the HTTP client for the model.
pub struct Localflow {
    pub cfg: Config,
    agent: ureq::Agent,
    /// Reasoning models (OpenAI's o-series, gpt-5) reject temperature and
    /// max_tokens; after the first such refusal, send only model and messages.
    api_minimal: AtomicBool,
}

impl Localflow {
    pub fn new(cfg: Config) -> Localflow {
        // CPU-only machines take ~20 s on a long dictation. Some providers'
        // firewalls block clients without a User-Agent.
        let agent = ureq::AgentBuilder::new().timeout(Duration::from_secs(120)).user_agent("localflow").build();
        Localflow { cfg, agent, api_minimal: AtomicBool::new(false) }
    }
}
