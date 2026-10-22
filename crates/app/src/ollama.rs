//! Ollama on this computer, for cleanup that never leaves it. When it isn't
//! there, localflow gets it: the official build from Ollama's latest GitHub
//! release, checked against the release's sha256sum.txt, installed, started,
//! and then the model pulled.
//!
//! Windows: OllamaSetup.exe, run silently (a per-user install; Ollama starts
//! itself at sign-in from then on). macOS: Ollama.app into /Applications (or
//! ~/Applications), opened in the background; it adds its own login item.
//! Linux: the release's tarball in ~/.local/share/localflow/ollama, run by a
//! systemd user service, or by localflow itself when you dictate.

use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::AtomicBool;
use std::time::{Duration, Instant};

use localflow_core::{Config, Localflow};
use sha2::{Digest, Sha256};

const RELEASE: &str = "https://github.com/ollama/ollama/releases/latest/download";

/// What each step reports, for the progress line.
pub const DOWNLOADING: &str = "Downloading Ollama";
pub const INSTALLING: &str = "Installing Ollama";
pub const STARTING: &str = "Starting Ollama";
pub const PULLING: &str = "Downloading";

/// Ollama's build for this computer.
fn asset() -> &'static str {
    if cfg!(windows) {
        "OllamaSetup.exe"
    } else if cfg!(target_os = "macos") {
        "Ollama-darwin.zip"
    } else if cfg!(target_arch = "aarch64") {
        "ollama-linux-arm64.tar.zst"
    } else {
        "ollama-linux-amd64.tar.zst"
    }
}

/// Is the configured Ollama this computer's? Only then is it ours to install.
pub fn is_local(cfg: &Config) -> bool {
    let host = cfg.ollama.split("://").nth(1).unwrap_or(&cfg.ollama);
    ["127.0.0.1", "localhost", "[::1]", "0.0.0.0"].iter().any(|h| host.starts_with(h))
}

/// Get Ollama going with `model`: install it if it isn't here, start it if
/// it isn't running, pull the model if it doesn't have it. `report` gets
/// (step, done, total).
pub fn get_ready(lf: &Localflow, model: &str, report: &mut dyn FnMut(&'static str, u64, u64)) -> Result<(), String> {
    if lf.ollama_models().is_err() {
        if !is_local(&lf.cfg) {
            return Err(format!("Ollama isn't running at {}", lf.cfg.ollama));
        }
        // Ollama's installer starts it on Windows and macOS: give it a moment.
        let fresh = installed().is_none();
        if fresh {
            install(report)?;
        }
        report(STARTING, 0, 0);
        if !(fresh && wait_up(lf, Duration::from_secs(10)).is_ok()) {
            start()?;
            wait_up(lf, Duration::from_secs(90))?;
        }
    }
    let have = lf.ollama_models().unwrap_or_default();
    if !have.iter().any(|m| m == model || m.strip_suffix(":latest") == Some(model)) {
        lf.ollama_pull(model, &mut |done, total| report(PULLING, done, total), &AtomicBool::new(false)).map_err(|e| e.0)?;
    }
    Ok(())
}

/// Dictating while Ollama is installed but not running (Linux without the
/// service, or someone quit it): start it, and wait a little for it.
pub fn wake(lf: &Localflow) -> bool {
    if !is_local(&lf.cfg) || installed().is_none() || start().is_err() {
        return false;
    }
    wait_up(lf, Duration::from_secs(8)).is_ok()
}

pub fn is_installed() -> bool {
    installed().is_some()
}

fn wait_up(lf: &Localflow, within: Duration) -> Result<(), String> {
    let since = Instant::now();
    while lf.ollama_models().is_err() {
        if since.elapsed() > within {
            return Err("Ollama is installed but didn't start".into());
        }
        std::thread::sleep(Duration::from_millis(500));
    }
    Ok(())
}

fn on_path(name: &str) -> Option<PathBuf> {
    std::env::split_paths(&std::env::var_os("PATH")?).map(|dir| dir.join(name)).find(|p| p.is_file())
}

/// Where localflow keeps its own Ollama on Linux.
fn linux_dir() -> PathBuf {
    crate::setup::linux_app().parent().unwrap().parent().unwrap().join("ollama")
}

/// What starts Ollama here, if it's installed.
fn installed() -> Option<PathBuf> {
    if cfg!(windows) {
        let programs = PathBuf::from(std::env::var_os("LOCALAPPDATA")?).join("Programs/Ollama");
        [programs.join("ollama app.exe"), programs.join("ollama.exe")]
            .into_iter()
            .find(|p| p.is_file())
            .or_else(|| on_path("ollama.exe"))
    } else if cfg!(target_os = "macos") {
        [PathBuf::from("/Applications/Ollama.app"), localflow_core::home().join("Applications/Ollama.app")]
            .into_iter()
            .find(|p| p.is_dir())
            .or_else(|| on_path("ollama"))
    } else {
        Some(linux_dir().join("bin/ollama")).filter(|p| p.is_file()).or_else(|| on_path("ollama"))
    }
}

/// Start Ollama in the background, without a window.
fn start() -> Result<(), String> {
    let path = installed().ok_or("Ollama isn't installed")?;
    if cfg!(target_os = "linux") && path.starts_with(linux_dir()) && service_installed() {
        let started = Command::new("systemctl").args(["--user", "start", SERVICE]).stdout(Stdio::null()).stderr(Stdio::null()).status();
        if started.is_ok_and(|s| s.success()) {
            return Ok(());
        }
    }
    let mut cmd = if path.extension().is_some_and(|e| e == "app") {
        let mut open = Command::new("open");
        open.args(["-g", "-j", "-a"]).arg(&path);
        open
    } else if path.file_name().is_some_and(|n| n == "ollama app.exe") {
        Command::new(&path)
    } else {
        let mut serve = Command::new(&path);
        serve.arg("serve");
        serve
    };
    cmd.stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null());
    #[cfg(windows)]
    std::os::windows::process::CommandExt::creation_flags(&mut cmd, 0x0800_0000 | 0x0000_0200); // no window, new group
    #[cfg(unix)]
    std::os::unix::process::CommandExt::process_group(&mut cmd, 0); // outlives the dictation that started it
    cmd.spawn().map(drop).map_err(|e| format!("couldn't start Ollama: {e}"))
}

/// Download Ollama's build, check it, and install it.
fn install(report: &mut dyn FnMut(&'static str, u64, u64)) -> Result<(), String> {
    let name = asset();
    let agent = ureq::AgentBuilder::new()
        .timeout_connect(Duration::from_secs(20))
        .timeout_read(Duration::from_secs(60))
        .user_agent("localflow")
        .build();
    let sums = agent
        .get(&format!("{RELEASE}/sha256sum.txt"))
        .call()
        .map_err(|e| format!("couldn't reach Ollama's releases: {e}"))?
        .into_string()
        .map_err(|e| e.to_string())?;
    let want = checksum(&sums, name).ok_or_else(|| format!("Ollama's release has no {name}"))?;
    let dir = downloads();
    fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let file = dir.join(name);
    if !file.exists() || sha256(&file).ok().as_deref() != Some(want.as_str()) {
        let part = dir.join(format!("{name}.part"));
        let got = fetch(&agent, &format!("{RELEASE}/{name}"), &part, report)?;
        if got != want {
            let _ = fs::remove_file(&part);
            return Err("Ollama's download didn't match its checksum; try again".into());
        }
        fs::rename(&part, &file).map_err(|e| e.to_string())?;
    }
    report(INSTALLING, 0, 0);
    let result = place(&file, report);
    let _ = fs::remove_file(&file);
    result
}

/// `name`'s line in a sha256sum.txt ("<hash>  ./<name>").
fn checksum(sums: &str, name: &str) -> Option<String> {
    sums.lines().find_map(|l| l.split_once(char::is_whitespace).filter(|(_, n)| n.trim().trim_start_matches("./") == name).map(|(h, _)| h.to_lowercase()))
}

/// Where the download waits until it's installed.
fn downloads() -> PathBuf {
    if cfg!(windows) {
        std::env::temp_dir().join("localflow")
    } else if cfg!(target_os = "macos") {
        localflow_core::home().join("Library/Caches/localflow")
    } else {
        linux_dir().with_file_name("downloads")
    }
}

fn sha256(path: &Path) -> std::io::Result<String> {
    let mut hasher = Sha256::new();
    std::io::copy(&mut File::open(path)?, &mut hasher)?;
    Ok(hex(&hasher.finalize()))
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Download `url` into `part`, going on from what an earlier try got.
/// Returns the whole file's sha256.
fn fetch(agent: &ureq::Agent, url: &str, part: &Path, report: &mut dyn FnMut(&'static str, u64, u64)) -> Result<String, String> {
    let io = |e: std::io::Error| format!("{}: {e}", part.display());
    let mut hasher = Sha256::new();
    let mut have = 0u64;
    if let Ok(mut f) = File::open(part) {
        have = std::io::copy(&mut f, &mut hasher).map_err(io)?;
    }
    let mut req = agent.get(url);
    if have > 0 {
        req = req.set("Range", &format!("bytes={have}-"));
    }
    let resp = match req.call() {
        // Already all here.
        Err(ureq::Error::Status(416, _)) => return Ok(hex(&hasher.finalize())),
        r => r.map_err(|e| format!("couldn't download Ollama: {e}"))?,
    };
    let resumed = have > 0 && resp.status() == 206;
    let rest: u64 = resp.header("Content-Length").and_then(|v| v.parse().ok()).unwrap_or(0);
    let mut out = if resumed {
        OpenOptions::new().append(true).open(part).map_err(io)?
    } else {
        hasher = Sha256::new();
        have = 0;
        File::create(part).map_err(io)?
    };
    let total = have + rest;
    let mut body = resp.into_reader();
    let mut buf = vec![0u8; 1 << 16];
    report(DOWNLOADING, have, total);
    loop {
        let n = body.read(&mut buf).map_err(|e| format!("Ollama's download stopped: {e}"))?;
        if n == 0 {
            break;
        }
        out.write_all(&buf[..n]).map_err(io)?;
        hasher.update(&buf[..n]);
        have += n as u64;
        report(DOWNLOADING, have, total);
    }
    if rest > 0 && have < total {
        return Err("Ollama's download stopped early; try again".into());
    }
    Ok(hex(&hasher.finalize()))
}

/// Install the checked download.
#[cfg(windows)]
fn place(setup: &Path, _: &mut dyn FnMut(&'static str, u64, u64)) -> Result<(), String> {
    // Ollama's own install script leaves this marker so the app starts in
    // the tray, without opening its window.
    if let Some(local) = std::env::var_os("LOCALAPPDATA") {
        let marker = PathBuf::from(local).join("Ollama");
        let _ = fs::create_dir_all(&marker);
        let _ = fs::write(marker.join("upgraded"), "");
    }
    let mut cmd = Command::new(setup);
    cmd.args(["/VERYSILENT", "/NORESTART", "/SUPPRESSMSGBOXES"]).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null());
    let status = cmd.status().map_err(|e| format!("couldn't run Ollama's installer: {e}"))?;
    if !status.success() {
        return Err(format!("Ollama's installer failed ({status})"));
    }
    Ok(())
}

#[cfg(target_os = "macos")]
fn place(zip: &Path, _: &mut dyn FnMut(&'static str, u64, u64)) -> Result<(), String> {
    let unpacked = zip.with_extension("unpacked");
    let _ = fs::remove_dir_all(&unpacked);
    let ok = Command::new("ditto").arg("-x").arg("-k").arg(zip).arg(&unpacked).status().is_ok_and(|s| s.success());
    let app = unpacked.join("Ollama.app");
    if !ok || !app.is_dir() {
        return Err("couldn't unpack Ollama".into());
    }
    let user_apps = localflow_core::home().join("Applications");
    let placed = [PathBuf::from("/Applications"), user_apps.clone()].into_iter().any(|dir| {
        if dir == user_apps {
            let _ = fs::create_dir_all(&dir);
        }
        let dest = dir.join("Ollama.app");
        fs::rename(&app, &dest).is_ok() || Command::new("ditto").arg(&app).arg(&dest).status().is_ok_and(|s| s.success())
    });
    let _ = fs::remove_dir_all(&unpacked);
    if placed { Ok(()) } else { Err("couldn't put Ollama in Applications".into()) }
}

#[cfg(target_os = "linux")]
fn place(tarball: &Path, report: &mut dyn FnMut(&'static str, u64, u64)) -> Result<(), String> {
    /// Counts what's read, for the progress line.
    struct Counted<'a, R> {
        inner: R,
        read: u64,
        total: u64,
        report: &'a mut dyn FnMut(&'static str, u64, u64),
    }
    impl<R: Read> Read for Counted<'_, R> {
        fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
            let n = self.inner.read(buf)?;
            self.read += n as u64;
            (self.report)(INSTALLING, self.read, self.total);
            Ok(n)
        }
    }
    let dir = linux_dir();
    let new = dir.with_file_name("ollama.new");
    let _ = fs::remove_dir_all(&new);
    fs::create_dir_all(&new).map_err(|e| format!("{}: {e}", new.display()))?;
    let file = File::open(tarball).map_err(|e| e.to_string())?;
    let total = file.metadata().map(|m| m.len()).unwrap_or(0);
    let counted = Counted { inner: file, read: 0, total, report };
    let mut zstd = zstd::stream::read::Decoder::new(counted).map_err(|e| e.to_string())?;
    zstd.window_log_max(31).map_err(|e| e.to_string())?;
    tar::Archive::new(zstd).unpack(&new).map_err(|e| format!("couldn't unpack Ollama: {e}"))?;
    if !new.join("bin/ollama").is_file() {
        return Err("Ollama's download has no bin/ollama".into());
    }
    let _ = fs::remove_dir_all(&dir);
    fs::rename(&new, &dir).map_err(|e| e.to_string())?;
    add_service(&dir.join("bin/ollama"));
    Ok(())
}

const SERVICE: &str = "localflow-ollama.service";

fn service_file() -> PathBuf {
    let config = std::env::var_os("XDG_CONFIG_HOME").map(PathBuf::from).unwrap_or_else(|| localflow_core::home().join(".config"));
    config.join("systemd/user").join(SERVICE)
}

fn service_installed() -> bool {
    service_file().is_file()
}

/// Linux: a systemd user service, so Ollama runs from login on. Without
/// systemd, dictating starts it (see `wake`).
#[cfg(target_os = "linux")]
fn add_service(bin: &Path) {
    if on_path("systemctl").is_none() {
        return;
    }
    let file = service_file();
    let unit = format!(
        "[Unit]\nDescription=Ollama, for localflow's cleanup\nAfter=network-online.target\n\n\
         [Service]\nExecStart=\"{}\" serve\nRestart=on-failure\nRestartSec=3\n\n\
         [Install]\nWantedBy=default.target\n",
        bin.display()
    );
    if fs::create_dir_all(file.parent().unwrap()).and_then(|_| fs::write(&file, unit)).is_err() {
        return;
    }
    let quiet = |args: &[&str]| Command::new("systemctl").args(args).stdout(Stdio::null()).stderr(Stdio::null()).status().is_ok_and(|s| s.success());
    if !(quiet(&["--user", "daemon-reload"]) && quiet(&["--user", "enable", SERVICE])) {
        let _ = fs::remove_file(&file);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_its_checksum() {
        let sums = "f2e5  ./ollama-linux-arm64.tar.zst\n0BB1  ./ollama-linux-amd64.tar.zst\n7777  ./OllamaSetup.exe\n";
        assert_eq!(checksum(sums, "ollama-linux-amd64.tar.zst").as_deref(), Some("0bb1"));
        assert_eq!(checksum(sums, "OllamaSetup.exe").as_deref(), Some("7777"));
        assert_eq!(checksum(sums, "ollama-linux-amd64"), None);
    }

    #[test]
    fn only_installs_for_this_computer() {
        let at = |url: &str| Config { ollama: url.into(), ..Config::from_vars(&|_| None, "/nowhere".into()) };
        assert!(is_local(&at("http://127.0.0.1:11434")));
        assert!(is_local(&at("http://localhost:11434")));
        assert!(!is_local(&at("http://gpu-box.lan:11434")));
    }

    /// The Linux build's layout (bin/ollama, lib/ollama/...), packed the
    /// way Ollama packs it, lands in localflow's folder.
    #[cfg(target_os = "linux")]
    #[test]
    fn unpacks_the_linux_build() {
        let root = std::env::temp_dir().join(format!("localflow-ollama-test-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        // Only this test touches the environment; no PATH keeps systemctl out of it.
        unsafe {
            std::env::set_var("XDG_DATA_HOME", root.join("data"));
            std::env::set_var("PATH", "");
        }
        let mut tar = tar::Builder::new(zstd::stream::write::Encoder::new(Vec::new(), 3).unwrap());
        for (path, body) in [("bin/ollama", &b"#!/bin/sh\n"[..]), ("lib/ollama/libggml-base.so", &b"lib"[..])] {
            let mut header = tar::Header::new_gnu();
            header.set_size(body.len() as u64);
            header.set_mode(0o755);
            header.set_cksum();
            tar.append_data(&mut header, path, body).unwrap();
        }
        let packed = tar.into_inner().unwrap().finish().unwrap();
        let tarball = root.join("ollama-linux-amd64.tar.zst");
        fs::write(&tarball, &packed).unwrap();
        let mut seen = 0;
        place(&tarball, &mut |step, done, _| {
            assert_eq!(step, INSTALLING);
            seen = done;
        })
        .unwrap();
        assert_eq!(seen, packed.len() as u64);
        let bin = root.join("data/localflow/ollama/bin/ollama");
        assert!(bin.is_file(), "{} is missing", bin.display());
        assert_eq!(installed(), Some(bin));
        assert!(root.join("data/localflow/ollama/lib/ollama/libggml-base.so").is_file());
        let _ = fs::remove_dir_all(&root);
    }

    /// A small file from the release, the second half after an interruption.
    #[test]
    #[ignore = "network"]
    fn downloads_and_resumes() {
        let agent = ureq::AgentBuilder::new().user_agent("localflow").build();
        let sums = agent.get(&format!("{RELEASE}/sha256sum.txt")).call().unwrap().into_string().unwrap();
        let want = checksum(&sums, "install.sh").unwrap();
        let part = std::env::temp_dir().join(format!("localflow-fetch-test-{}.part", std::process::id()));
        let whole = agent.get(&format!("{RELEASE}/install.sh")).call().unwrap().into_string().unwrap();
        fs::write(&part, &whole.as_bytes()[..1000]).unwrap();
        let mut last = (0, 0);
        let got = fetch(&agent, &format!("{RELEASE}/install.sh"), &part, &mut |_, done, total| last = (done, total)).unwrap();
        assert_eq!(got, want);
        assert_eq!(last, (whole.len() as u64, whole.len() as u64));
        assert_eq!(fs::read(&part).unwrap(), whole.as_bytes());
        let _ = fs::remove_file(&part);
    }
}
