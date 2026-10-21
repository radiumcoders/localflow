//! What you said and what went out, for `localflow history`: the last
//! dictations, in a file only you can read, next to the log (which never
//! holds what you said). LOCALFLOW_HISTORY=off keeps none.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::{Value, json};

use crate::paths::{info, state_dir};

/// How many dictations it keeps.
const KEEP: usize = 200;

pub fn history_file() -> PathBuf {
    state_dir().join("history.jsonl")
}

/// One dictation.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Entry {
    /// Seconds since 1970.
    pub time: f64,
    /// The app it went to, and the coding agent running there, if any.
    pub app: String,
    pub agent: Option<String>,
    /// The transcript, as heard.
    pub heard: String,
    /// What went out: the cleaned text, or the keys a command pressed.
    pub text: String,
    /// "pasted", "sent", "copied" (left on the clipboard) or "command".
    pub outcome: String,
}

impl Entry {
    fn to_json(&self) -> Value {
        json!({"time": self.time, "app": self.app, "agent": self.agent, "heard": self.heard, "text": self.text,
               "outcome": self.outcome})
    }

    fn from_json(v: &Value) -> Option<Entry> {
        let text = |k: &str| v[k].as_str().unwrap_or_default().to_string();
        Some(Entry {
            time: v["time"].as_f64()?,
            app: text("app"),
            agent: v["agent"].as_str().map(String::from),
            heard: text("heard"),
            text: text("text"),
            outcome: text("outcome"),
        })
    }
}

/// Keep a dictation (stamped now) in the history.
pub fn add(mut entry: Entry) {
    entry.time = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0.0, |d| d.as_secs_f64());
    if let Err(e) = add_at(&history_file(), &entry) {
        info(format_args!("couldn't keep the dictation in the history: {e}"));
    }
}

/// Add `entry` to the history at `path`, dropping the oldest beyond KEEP.
pub fn add_at(path: &Path, entry: &Entry) -> std::io::Result<()> {
    let old = std::fs::read_to_string(path).unwrap_or_default();
    let mut lines: Vec<String> = old.lines().filter(|l| !l.trim().is_empty()).map(String::from).collect();
    lines.push(entry.to_json().to_string());
    let keep = &lines[lines.len().saturating_sub(KEEP)..];
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let tmp = path.with_extension(format!("{}.tmp", std::process::id()));
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
    options.open(&tmp)?.write_all((keep.join("\n") + "\n").as_bytes())?;
    std::fs::rename(&tmp, path)
}

/// The last `n` dictations, oldest first.
pub fn recent(n: usize) -> Vec<Entry> {
    recent_at(&history_file(), n)
}

pub fn recent_at(path: &Path, n: usize) -> Vec<Entry> {
    let text = std::fs::read_to_string(path).unwrap_or_default();
    let all: Vec<Entry> =
        text.lines().filter_map(|l| serde_json::from_str::<Value>(l).ok()).filter_map(|v| Entry::from_json(&v)).collect();
    all[all.len().saturating_sub(n)..].to_vec()
}

/// Forget every dictation.
pub fn clear() -> std::io::Result<()> {
    match std::fs::remove_file(history_file()) {
        Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e),
        _ => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_the_last_ones() {
        let dir = std::env::temp_dir().join(format!("localflow-history-{}", std::process::id()));
        let path = dir.join("history.jsonl");
        let _ = std::fs::remove_dir_all(&dir);
        assert!(recent_at(&path, 5).is_empty());
        for i in 0..KEEP + 3 {
            let e = Entry { time: i as f64, heard: format!("heard {i}"), text: format!("Text {i}."), ..Entry::default() };
            add_at(&path, &e).unwrap();
        }
        let last = recent_at(&path, 2);
        assert_eq!(last.iter().map(|e| e.text.as_str()).collect::<Vec<_>>(), [
            format!("Text {}.", KEEP + 1),
            format!("Text {}.", KEEP + 2)
        ]);
        assert_eq!(recent_at(&path, 10_000).len(), KEEP);
        assert_eq!(recent_at(&path, 10_000)[0].time, 3.0);
        let sent = Entry { agent: Some("claude".into()), outcome: "sent".into(), ..Entry::default() };
        add_at(&path, &sent).unwrap();
        assert_eq!(recent_at(&path, 1), [sent]);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(std::fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o600);
        }
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
