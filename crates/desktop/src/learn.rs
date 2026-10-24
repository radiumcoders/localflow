//! Learning from your corrections. The last pasted text is kept (readable only
//! by you, in the run folder) so your fixed version can be compared with it
//! when you press the learn key: the words you changed are added to your
//! vocabulary ("Claw" -> "Claude"). Nothing is logged.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use localflow_core::{Localflow, corrections};
use serde_json::{Value, json};

use crate::paths::{info, make_run_dir, run_dir};

/// LOCALFLOW_LEARN: with the learn key, or never.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Learning {
    Key,
    Off,
}

/// LOCALFLOW_LEARN from the environment or the env file ("key" by default).
pub fn learning_mode() -> Learning {
    let from_file = || {
        let text = std::fs::read_to_string(localflow_core::config_dir().join("env")).ok()?;
        text.lines().filter_map(|l| l.trim().split_once('=')).find(|(k, _)| *k == "LOCALFLOW_LEARN").map(|(_, v)| v.to_string())
    };
    let value = std::env::var("LOCALFLOW_LEARN").ok().or_else(from_file).unwrap_or_default();
    if value.trim().eq_ignore_ascii_case("off") { Learning::Off } else { Learning::Key }
}

fn now() -> f64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0.0, |d| d.as_secs_f64())
}

pub fn last_file() -> PathBuf {
    run_dir().join("last.json")
}

/// Keep `text` as the last dictation. Returns its time stamp, which tells
/// a watcher whether a newer dictation took over.
pub fn remember(text: &str) -> f64 {
    let stamp = now();
    if let Err(e) = make_run_dir().and_then(|_| remember_at(&last_file(), text, stamp)) {
        info(format_args!("couldn't keep the last dictation: {e}"));
    }
    stamp
}

/// Write the last dictation to `path` (only you can read it on Unix).
pub fn remember_at(path: &Path, text: &str, stamp: f64) -> std::io::Result<()> {
    let tmp = path.with_extension(format!("{}.tmp", std::process::id()));
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
    options.open(&tmp)?.write_all(json!({"text": text, "time": stamp}).to_string().as_bytes())?;
    std::fs::rename(&tmp, path)
}

/// (text, time stamp) of the last dictation.
pub fn last_dictation() -> Option<(String, f64)> {
    last_dictation_at(&last_file())
}

pub fn last_dictation_at(path: &Path) -> Option<(String, f64)> {
    let d: Value = serde_json::from_str(&std::fs::read_to_string(path).ok()?).ok()?;
    Some((d["text"].as_str()?.to_string(), d["time"].as_f64()?))
}

/// What the overlay says after learning.
pub fn learned_message(added: &[(String, String)]) -> String {
    let (heard, meant) = &added[0];
    let shown = format!("Learned: {heard} → {meant}");
    if added.len() == 1 && shown.chars().count() <= 30 {
        shown
    } else {
        format!("Learned {} word{}", added.len(), if added.len() > 1 { "s" } else { "" })
    }
}

/// The learn key: compare the text you fixed (selected or copied) with the
/// last dictation and remember the words you changed. Returns a message.
pub fn learn_correction(lf: &Localflow) -> String {
    let Some((last, _)) = last_dictation() else { return "Dictate something first".into() };
    learn_from(lf, &last, crate::paste::selected_text())
}

/// The learn key's work, given the last dictation and the texts that may be
/// your fixed version of it (most likely first).
pub fn learn_from(lf: &Localflow, last: &str, candidates: Vec<String>) -> String {
    let Some(pairs) = candidates.iter().filter(|t| !t.trim().is_empty()).find_map(|t| corrections(last, t)) else {
        return "Select the text you fixed".into();
    };
    if pairs.is_empty() {
        return "No changed words found".into();
    }
    match lf.learn(&pairs) {
        Ok(added) => {
            info(format_args!("learned {} of {} corrections", added.len(), pairs.len())); // not the words
            if added.is_empty() { "Already learned".into() } else { learned_message(&added) }
        }
        Err(e) => format!("Couldn't save: {e}"),
    }
}

/// (pid, text) of the text box you're typing in: macOS and Windows (Linux
/// apps don't expose theirs).
pub fn focused_text() -> Option<(u32, String)> {
    #[cfg(windows)]
    return crate::windows::focused_text();
    #[cfg(target_os = "macos")]
    return crate::macos::focused_text();
    #[cfg(not(any(windows, target_os = "macos")))]
    None
}
