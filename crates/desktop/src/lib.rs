//! localflow's desktop plumbing: where
//! you're dictating (the app, its project, the files you may name), pasting
//! and the clipboard, the hotkey on Windows and macOS, and learning from the
//! fixes you make to what was pasted. No UI: the app crate draws the overlay.

pub mod history;
pub mod hotkey;
pub mod learn;
pub mod paste;
pub mod paths;
pub mod project;
pub mod target;

#[cfg(target_os = "macos")]
mod macos;
#[cfg(windows)]
mod windows;

use std::path::PathBuf;
use std::process::{Command, Output, Stdio};
use std::time::{Duration, Instant};

pub use localflow_core::Part;

/// A program on PATH (with .exe on Windows).
pub fn which(name: &str) -> Option<PathBuf> {
    let exts: &[&str] = if cfg!(windows) { &[".exe", ""] } else { &[""] };
    std::env::split_paths(&std::env::var_os("PATH")?)
        .flat_map(|dir| exts.iter().map(move |ext| dir.join(format!("{name}{ext}"))))
        .find(|p| p.is_file())
}

/// A command for a helper program: no console window flashing up on Windows.
pub(crate) fn command(program: impl AsRef<std::ffi::OsStr>) -> Command {
    #[allow(unused_mut)]
    let mut cmd = Command::new(program);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }
    cmd
}

/// Run to completion with its output captured, or None if it can't start or
/// takes longer than `limit` (then it's killed).
pub(crate) fn run(cmd: &mut Command, limit: Duration) -> Option<Output> {
    let mut child = cmd.stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn().ok()?;
    // Read both pipes on threads so a chatty child can't fill one and stall.
    let (mut out, mut err) = (child.stdout.take()?, child.stderr.take()?);
    let out = std::thread::spawn(move || {
        let mut buf = vec![];
        let _ = std::io::Read::read_to_end(&mut out, &mut buf);
        buf
    });
    let err = std::thread::spawn(move || {
        let mut buf = vec![];
        let _ = std::io::Read::read_to_end(&mut err, &mut buf);
        buf
    });
    let start = Instant::now();
    let status = loop {
        if let Some(status) = child.try_wait().ok()? {
            break status;
        }
        if start.elapsed() > limit {
            let _ = child.kill();
            let _ = child.wait();
            return None;
        }
        std::thread::sleep(Duration::from_millis(10));
    };
    Some(Output { status, stdout: out.join().ok()?, stderr: err.join().ok()? })
}

/// A command's standard output as text, "" if it failed to run.
pub(crate) fn output(cmd: &mut Command, limit: Duration) -> String {
    run(cmd, limit).map(|o| String::from_utf8_lossy(&o.stdout).into_owned()).unwrap_or_default()
}

/// A desktop notification, for when no overlay is running (the Linux
/// watcher, or a machine without one). `ms` 0: until dismissed.
pub fn notify(text: &str, ms: u32) {
    let mut cmd = if cfg!(target_os = "macos") {
        let quoted = text.replace('\\', "\\\\").replace('"', "\\\"");
        let mut c = command("osascript");
        c.arg("-e").arg(format!("display notification \"{quoted}\" with title \"localflow\""));
        c
    } else if cfg!(target_os = "linux") && which("notify-send").is_some() {
        let mut c = command("notify-send");
        c.args(["-a", "localflow", "-t", &ms.to_string(), text]);
        c
    } else {
        return;
    };
    let _ = cmd.stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).spawn();
}
