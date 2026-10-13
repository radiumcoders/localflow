//! localflow's speech input: record the default microphone as 16 kHz mono
//! (what Handy's speech engine takes), show its level, and turn it into text
//! with Handy's headless `--transcribe-file`, the same engine and model as
//! Handy's app, loaded only while it runs.

mod audio;
mod handy;
mod recorder;

use std::fmt;

pub use audio::{RATE, level, read_wav, resample, wav_bytes};
pub use handy::{handy_path, run_dir, transcribe};
pub use recorder::Recorder;

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
