//! Learning from your corrections. The last pasted text is kept (readable only
//! by you, in the run folder) so your fixed version can be compared with it:
//! when you press the learn key, or on its own for a few minutes after each
//! paste, from the message you send to a coding agent or Cursor's chat (their
//! history files, on every OS) or, on macOS and Windows, the text box itself.
//! The words you changed go to the cleanup model, which keeps the fixes of
//! misheard words ("Claw" -> "Claude") and drops changes of mind ("Tuesday"
//! -> "Wednesday"). Nothing is logged.

use std::collections::HashMap;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use localflow_core::{Localflow, corrections, edited_region};
use serde_json::{Value, json};

use crate::paths::{info, make_run_dir, run_dir};
use crate::project::{EDITORS, editor_base, editor_state_db, open_db};

/// How long after a paste dictate watches for your fixes.
pub const WATCH: Duration = Duration::from_secs(180);

/// LOCALFLOW_LEARN: on its own and with the key, only with the key, or never.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Learning {
    Auto,
    Key,
    Off,
}

/// LOCALFLOW_LEARN from the environment or the env file ("auto" by default).
pub fn learning_mode() -> Learning {
    let from_file = || {
        let text = std::fs::read_to_string(localflow_core::config_dir().join("env")).ok()?;
        text.lines().filter_map(|l| l.trim().split_once('=')).find(|(k, _)| *k == "LOCALFLOW_LEARN").map(|(_, v)| v.to_string())
    };
    let value = std::env::var("LOCALFLOW_LEARN").ok().or_else(from_file).unwrap_or_else(|| "auto".into());
    match value.trim().to_lowercase().as_str() {
        "auto" => Learning::Auto,
        "off" => Learning::Off,
        _ => Learning::Key,
    }
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

/// The fixes you made, judged by the model; the mishearings are learned and
/// `notify`d.
pub fn learn_on_its_own(lf: &Localflow, pasted: &str, edited: &str, notify: &dyn Fn(String)) {
    let Some(pairs) = corrections(pasted, edited).filter(|p| !p.is_empty()) else { return };
    let keep = match lf.mishearings(pasted, edited, &pairs, &lf.cfg.model) {
        Ok(keep) => keep,
        Err(e) => return info(format_args!("couldn't check the changes: {e}")),
    };
    match lf.learn(&keep) {
        Ok(added) => {
            info(format_args!("learned {} of {} changes you made", added.len(), pairs.len())); // not the words
            if !added.is_empty() {
                notify(learned_message(&added));
            }
        }
        Err(e) => info(format_args!("couldn't save what was learned: {e}")),
    }
}

/// Messages you send from now on: to Claude Code and Codex (their
/// transcripts) and in Cursor's chat (its database).
pub struct SentMessages {
    started: f64,
    logs: Vec<PathBuf>,
    offsets: HashMap<PathBuf, u64>,
    chats: HashMap<PathBuf, i64>,
}

const LOGS: &[&str] = &[".claude/projects", ".codex/sessions"];

impl SentMessages {
    /// Watching your home folder's transcripts and every editor's chat.
    pub fn start() -> SentMessages {
        let base = editor_base();
        let mut names: Vec<&str> = EDITORS.iter().map(|(_, n)| *n).collect();
        names.sort();
        names.dedup();
        let dbs = names.into_iter().map(|n| editor_state_db(&base, n)).collect();
        SentMessages::watching(&localflow_core::home(), dbs)
    }

    /// Watching the transcripts under `home` and the chats in `dbs`.
    pub fn watching(home: &Path, dbs: Vec<PathBuf>) -> SentMessages {
        let started = now();
        let logs = LOGS.iter().map(|rel| home.join(rel)).collect();
        let mut sent = SentMessages { started, logs, offsets: HashMap::new(), chats: HashMap::new() };
        sent.offsets = sent.transcripts(started - 86400.0).into_iter().collect();
        for db in dbs {
            let rowid = open_db(&db).and_then(|c| c.query_row("SELECT max(rowid) FROM cursorDiskKV", [], |r| r.get::<_, Option<i64>>(0)));
            if let Ok(rowid) = rowid {
                sent.chats.insert(db, rowid.unwrap_or(0)); // otherwise: no such editor, or no chat history in it
            }
        }
        sent
    }

    /// (path, size) of the transcripts changed since `since`.
    fn transcripts(&self, since: f64) -> Vec<(PathBuf, u64)> {
        let mut found = vec![];
        let mut todo: Vec<PathBuf> = self.logs.iter().filter(|d| d.is_dir()).cloned().collect();
        while let Some(dir) = todo.pop() {
            for entry in std::fs::read_dir(&dir).into_iter().flatten().flatten() {
                let path = entry.path();
                let Ok(meta) = entry.metadata() else { continue };
                if meta.is_dir() {
                    todo.push(path);
                } else if path.extension().is_some_and(|e| e == "jsonl") {
                    // The file's own size: on Windows a directory listing's is
                    // stale while the agent still has the file open.
                    let Ok(meta) = std::fs::metadata(&path) else { continue };
                    let mtime = meta.modified().ok().and_then(|t| t.duration_since(UNIX_EPOCH).ok()).map_or(0.0, |d| d.as_secs_f64());
                    if mtime >= since {
                        found.push((path, meta.len()));
                    }
                }
            }
        }
        found
    }

    /// Text you wrote, from one transcript line (Claude Code or Codex).
    fn user_text(entry: &Value) -> Vec<String> {
        let content = if entry["type"] == "user" && !entry["isMeta"].as_bool().unwrap_or(false) {
            &entry["message"]["content"] // Claude Code
        } else if entry["payload"]["role"] == "user" {
            &entry["payload"]["content"] // Codex
        } else {
            return vec![];
        };
        match content {
            Value::String(s) => vec![s.clone()],
            Value::Array(blocks) => blocks
                .iter()
                .filter(|b| b["type"] == "text" || b["type"] == "input_text")
                .map(|b| b["text"].as_str().unwrap_or_default().to_string())
                .collect(),
            _ => vec![],
        }
    }

    /// What you sent since the last call.
    pub fn take_new(&mut self) -> Vec<String> {
        let mut texts = vec![];
        for (path, size) in self.transcripts(self.started - 5.0) {
            let start = self.offsets.get(&path).copied().unwrap_or(0);
            if size <= start {
                continue;
            }
            let Ok(data) = read_range(&path, start, size) else { continue };
            // A line still being written is read next time.
            let done = data.iter().rposition(|&b| b == b'\n').map_or(0, |i| i + 1);
            self.offsets.insert(path, start + done as u64);
            for line in data[..done].split(|&b| b == b'\n') {
                if let Ok(entry) = serde_json::from_slice::<Value>(line) {
                    texts.extend(Self::user_text(&entry));
                }
            }
        }
        for (db, rowid) in self.chats.iter_mut() {
            let rows = open_db(db).and_then(|c| {
                let mut q = c.prepare("SELECT rowid, value FROM cursorDiskKV WHERE rowid > ?1 AND key LIKE 'bubbleId:%'")?;
                let rows = q.query_map([*rowid], |r| Ok((r.get::<_, i64>(0)?, r.get_ref(1)?.as_bytes().map(<[u8]>::to_vec).unwrap_or_default())))?;
                rows.collect::<rusqlite::Result<Vec<_>>>()
            });
            for (row, value) in rows.into_iter().flatten() {
                *rowid = (*rowid).max(row);
                let Ok(bubble) = serde_json::from_slice::<Value>(&value) else { continue };
                let mine = matches!(&bubble["type"], Value::Number(n) if n.as_i64() == Some(1)) || bubble["type"] == "1";
                match bubble["text"].as_str() {
                    Some(text) if mine && !text.is_empty() => texts.push(text.to_string()), // 1: a message you wrote
                    _ => {}
                }
            }
        }
        texts
    }
}

fn read_range(path: &Path, start: u64, end: u64) -> std::io::Result<Vec<u8>> {
    use std::io::{Read, Seek, SeekFrom};
    let mut f = std::fs::File::open(path)?;
    f.seek(SeekFrom::Start(start))?;
    let mut data = vec![];
    f.take(end - start).read_to_end(&mut data)?;
    Ok(data)
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

/// Where and how often `watch` looks.
pub struct Watch {
    pub sent: SentMessages,
    pub last_file: PathBuf,
    pub poll: Duration,
    pub limit: Duration,
    /// Reads the focused text box (`focused_text`, or a test's).
    pub text_box: fn() -> Option<(u32, String)>,
}

impl Watch {
    pub fn start() -> Watch {
        Watch { sent: SentMessages::start(), last_file: last_file(), poll: Duration::from_millis(1500), limit: WATCH, text_box: focused_text }
    }
}

/// Learn the words you fix in what was just pasted (see the module doc).
/// Stops after WATCH, at the next dictation, or once it has seen what you
/// kept. `pid`: the app it was pasted into.
pub fn watch(lf: &Localflow, pasted: &str, pid: Option<u32>, stamp: f64, notify: &dyn Fn(String)) {
    watch_with(lf, pasted, pid, stamp, notify, Watch::start());
}

pub fn watch_with(lf: &Localflow, pasted: &str, pid: Option<u32>, stamp: f64, notify: &dyn Fn(String), mut w: Watch) {
    let deadline = Instant::now() + w.limit;
    let mut kept: Option<String> = None;
    while Instant::now() < deadline {
        std::thread::sleep(w.poll);
        if last_dictation_at(&w.last_file).map(|(_, t)| t) != Some(stamp) {
            break; // a newer dictation took over
        }
        if let Some(edited) = w.sent.take_new().iter().find_map(|text| edited_region(pasted, text)) {
            return learn_on_its_own(lf, pasted, &edited, notify);
        }
        if let Some((box_pid, text)) = (w.text_box)()
            && pid == Some(box_pid)
            && let Some(edited) = edited_region(pasted, &text)
        {
            kept = Some(edited); // still there, maybe being edited
            continue;
        }
        if let Some(edited) = kept.take() {
            return learn_on_its_own(lf, pasted, &edited, notify); // sent, cleared, or you moved on
        }
    }
    if let Some(edited) = kept {
        learn_on_its_own(lf, pasted, &edited, notify);
    }
}

