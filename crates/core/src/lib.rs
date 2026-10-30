//! localflow's dictation cleanup.
//!
//! A raw speech-to-text transcript goes in; what the speaker meant to write
//! comes out: self-corrections applied, filler dropped, punctuation, lists and
//! headings, names spelled the way the speaker's vocabulary says. Runs
//! localflow's own model, localflow-clean, on this computer (in this process,
//! see local.rs): nothing is sent anywhere.
//!
//! ```no_run
//! let lf = localflow_core::Localflow::new(localflow_core::Config::from_env());
//! println!("{}", lf.clean("let's meet at 3 wait at 4", localflow_core::NO_FILES)?);
//! # Ok::<(), localflow_core::Error>(())
//! ```

pub mod agent;
mod cleanup;
mod config;
pub mod difflib;
pub mod eval;
pub mod local;
pub mod snippets;
mod vocab;

use std::fmt;
use std::sync::Mutex;

pub use cleanup::{Cleaned, Part, file_refs, markdown_headings, plausible, user_turn};
pub use config::{Config, config_dir, home, save_settings};
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

/// The cleanup engine: settings, plus the model once it's loaded.
pub struct Localflow {
    pub cfg: Config,
    /// The local model, once loaded; freed with this Localflow.
    engine: Mutex<Option<localflow_llm::Model>>,
}

impl Localflow {
    pub fn new(cfg: Config) -> Localflow {
        Localflow { cfg, engine: Mutex::new(None) }
    }
}
