//! Speech to text with Handy's engine: its headless `--transcribe-file`.

use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use serde_json::Value;

use crate::audio::{RATE, wav_bytes};
use crate::{Error, Result, err};

fn env(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|v| !v.is_empty())
}

fn home() -> PathBuf {
    PathBuf::from(env(if cfg!(windows) { "USERPROFILE" } else { "HOME" }).unwrap_or_default())
}

fn on_path(name: &str) -> Option<PathBuf> {
    let exts: &[&str] = if cfg!(windows) { &[".exe", ""] } else { &[""] };
    std::env::split_paths(&std::env::var_os("PATH")?)
        .flat_map(|dir| exts.iter().map(move |ext| dir.join(format!("{name}{ext}"))))
        .find(|p| p.is_file())
}

/// Handy's executable: DICTATE_HANDY, where Handy installs itself, or PATH.
pub fn handy_path() -> Option<PathBuf> {
    if let Some(path) = env("DICTATE_HANDY") {
        return Some(path.into());
    }
    let candidates: Vec<PathBuf> = if cfg!(target_os = "macos") {
        [PathBuf::from("/Applications/Handy.app"), home().join("Applications/Handy.app")]
            .into_iter()
            .filter_map(|app| {
                // The executable named in Info.plist (an XML plist for Handy).
                let plist = std::fs::read_to_string(app.join("Contents/Info.plist")).ok()?;
                let after = plist.split("<key>CFBundleExecutable</key>").nth(1)?;
                let name = after.split("<string>").nth(1)?.split("</string>").next()?.trim();
                Some(app.join("Contents/MacOS").join(name))
            })
            .collect()
    } else if cfg!(windows) {
        ["LOCALAPPDATA", "ProgramFiles"].iter().filter_map(|v| env(v)).map(|b| PathBuf::from(b).join("Handy/Handy.exe")).collect()
    } else {
        vec![home().join(".local/bin/handy")]
    };
    candidates.into_iter().find(|p| p.exists()).or_else(|| on_path("handy"))
}

/// Where a dictation's audio waits for Handy (and other per-session files).
pub fn run_dir() -> PathBuf {
    let base = if cfg!(target_os = "linux") {
        env("XDG_RUNTIME_DIR").map(PathBuf::from).unwrap_or_else(std::env::temp_dir)
    } else {
        std::env::temp_dir()
    };
    base.join("localflow-dictate")
}

/// Speech (16 kHz mono) to text. `model`: a Handy model id, or None for the
/// one selected in Handy.
pub fn transcribe(samples: &[i16], model: Option<&str>) -> Result<String> {
    let exe = handy_path().ok_or_else(|| Error("Handy not found (install it, or set DICTATE_HANDY)".into()))?;
    let dir = run_dir();
    std::fs::create_dir_all(&dir).map_err(|e| Error(format!("{}: {e}", dir.display())))?;
    let wav = dir.join(format!("audio-{}.wav", std::process::id()));
    std::fs::write(&wav, wav_bytes(samples, RATE)).map_err(|e| Error(format!("{}: {e}", wav.display())))?;
    let mut cmd = Command::new(&exe);
    cmd.arg("--transcribe-file").arg(&wav).arg("--json");
    if let Some(model) = model {
        cmd.arg("--model").arg(model);
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }
    let out = run(cmd.stdin(Stdio::null()), Duration::from_secs(120));
    let _ = std::fs::remove_file(&wav);
    let out = out?;
    match serde_json::from_slice::<Value>(&out.stdout) {
        Ok(answer) => Ok(answer["text"].as_str().unwrap_or_default().trim().to_string()),
        Err(_) => {
            let stderr = String::from_utf8_lossy(&out.stderr);
            let last = stderr.trim().lines().last().map(String::from).unwrap_or_else(|| format!("{}", out.status));
            err(format!("Handy couldn't transcribe: {last}"))
        }
    }
}

/// Run a command to completion, killing it after `limit`.
fn run(cmd: &mut Command, limit: Duration) -> Result<std::process::Output> {
    let mut child = cmd
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| Error(format!("couldn't run Handy: {e}")))?;
    // Read both pipes on threads so a chatty child can't fill one and stall.
    let mut stdout = child.stdout.take().unwrap();
    let mut stderr = child.stderr.take().unwrap();
    let out_thread = std::thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = std::io::Read::read_to_end(&mut stdout, &mut buf);
        buf
    });
    let err_thread = std::thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = std::io::Read::read_to_end(&mut stderr, &mut buf);
        buf
    });
    let start = Instant::now();
    let status = loop {
        if let Some(status) = child.try_wait().map_err(|e| Error(e.to_string()))? {
            break status;
        }
        if start.elapsed() > limit {
            let _ = child.kill();
            let _ = child.wait();
            return err("Handy took too long to transcribe");
        }
        std::thread::sleep(Duration::from_millis(20));
    };
    Ok(std::process::Output { status, stdout: out_thread.join().unwrap(), stderr: err_thread.join().unwrap() })
}
