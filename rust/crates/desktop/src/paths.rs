//! Where dictate keeps its log and its per-session files, and the log itself
//! (which never holds what you said).

use std::fmt::Display;
use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

use localflow_core::home;

fn env(name: &str) -> Option<PathBuf> {
    std::env::var_os(name).filter(|v| !v.is_empty()).map(PathBuf::from)
}

/// The log's folder: ~/.local/state/localflow, ~/Library/Logs/localflow,
/// %LOCALAPPDATA%\localflow.
pub fn state_dir() -> PathBuf {
    if cfg!(windows) {
        env("LOCALAPPDATA").unwrap_or_else(home).join("localflow")
    } else if cfg!(target_os = "macos") {
        home().join("Library/Logs/localflow")
    } else {
        env("XDG_STATE_HOME").unwrap_or_else(|| home().join(".local/state")).join("localflow")
    }
}

/// Per-session files (the running session, stop/cancel requests, the last
/// dictation): somewhere only you can read and that a reboot clears.
pub fn run_dir() -> PathBuf {
    let base = if cfg!(target_os = "linux") {
        env("XDG_RUNTIME_DIR").unwrap_or_else(std::env::temp_dir)
    } else if cfg!(target_os = "macos") {
        env("TMPDIR").unwrap_or_else(|| "/tmp".into())
    } else {
        std::env::temp_dir()
    };
    base.join("localflow-dictate")
}

/// The run folder, created (only you can enter it on Unix).
pub fn make_run_dir() -> std::io::Result<PathBuf> {
    let dir = run_dir();
    std::fs::create_dir_all(&dir)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700));
    }
    Ok(dir)
}

pub fn log_file() -> PathBuf {
    state_dir().join("dictate.log")
}

static LOG: Mutex<Option<File>> = Mutex::new(None);

/// Log to dictate.log as well as stderr. Background mode has no terminal; a
/// process whose stderr already goes to the log (Linux sessions) skips this.
pub fn init_log() -> std::io::Result<()> {
    std::fs::create_dir_all(state_dir())?;
    let file = OpenOptions::new().create(true).append(true).open(log_file())?;
    *LOG.lock().unwrap() = Some(file);
    Ok(())
}

pub fn info(msg: impl Display) {
    write_log("INFO", msg);
}

pub fn warn(msg: impl Display) {
    write_log("WARNING", msg);
}

pub fn error(msg: impl Display) {
    write_log("ERROR", msg);
}

fn write_log(level: &str, msg: impl Display) {
    let line = format!("{} {level} {msg}\n", timestamp());
    if let Some(file) = LOG.lock().unwrap().as_mut() {
        let _ = file.write_all(line.as_bytes());
    }
    let _ = std::io::stderr().write_all(line.as_bytes());
}

/// Now in UTC, as 2026-10-20T14:45:04Z (no time zone database needed).
fn timestamp() -> String {
    let secs = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs()) as i64;
    let (days, rem) = (secs.div_euclid(86400), secs.rem_euclid(86400));
    // Civil date from days since 1970-01-01 (Howard Hinnant's algorithm).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!("{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}Z", rem / 3600, rem % 3600 / 60, rem % 60)
}

#[cfg(test)]
mod tests {
    #[test]
    fn timestamp_shape() {
        let t = super::timestamp();
        assert_eq!(t.len(), 20, "{t}");
        assert!(t.starts_with("20") && t.ends_with('Z'), "{t}");
    }
}
