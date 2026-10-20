//! localflow's speech input: record the default microphone as 16 kHz mono,
//! show its level, and turn it into text with whisper.cpp, in this process.
//! No other app is needed: the speech model (a ggml file localflow downloads,
//! see [`models`]) is loaded for a dictation and dropped after it.
//!
//! ```no_run
//! use localflow_speech::{Recorder, SpeechConfig, transcribe};
//! let rec = Recorder::start(|_level| {})?;
//! std::thread::sleep(std::time::Duration::from_secs(3));
//! let heard = transcribe(&rec.stop(), &SpeechConfig::default())?;
//! println!("{}", heard.for_cleanup(true)); // quiet asides in <quiet> tags
//! # Ok::<(), localflow_speech::Error>(())
//! ```

mod audio;
mod engine;
pub mod models;
mod recorder;
mod transcript;

use std::fmt;
use std::path::PathBuf;

pub use audio::{RATE, level, read_wav, resample, wav_bytes};
pub use engine::{Engine, Options, SpeechConfig, transcribe};
pub use recorder::Recorder;
pub use transcript::{QUIET_DROP_DB, Segment, Transcript};

#[derive(Debug, Clone, PartialEq)]
pub struct Error(pub String);

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for Error {}

pub type Result<T> = std::result::Result<T, Error>;

pub(crate) fn err<T>(msg: impl Into<String>) -> Result<T> {
    Err(Error(msg.into()))
}

/// Where a dictation's per-session files go (the last dictation, markers
/// between `localflow start` and `stop`): private, cleared at logout on Linux.
pub fn run_dir() -> PathBuf {
    let base = if cfg!(target_os = "linux") {
        std::env::var("XDG_RUNTIME_DIR")
            .ok()
            .filter(|v| !v.is_empty())
            .map(PathBuf::from)
            .unwrap_or_else(std::env::temp_dir)
    } else {
        std::env::temp_dir()
    };
    base.join("localflow-dictate")
}
