//! How dictation runs: one short-lived process per dictation (`_session`),
//! which records right away, shows the pill, and exits once it has pasted.
//! `stop` and `cancel` leave a file for it (files, not signals: a quick tap
//! can send stop before the session has started).
//!
//! Linux: compositors own the keys, so key bindings run `localflow start` /
//! `stop`. Nothing runs between dictations.
//!
//! macOS and Windows: a background app (`--background`) waits for the
//! hotkey. It has no UI of its own (a few MB): the hotkey starts a session
//! process the same way. Opening localflow again opens the settings. It holds
//! 127.0.0.1:11437 while it runs, so only one of it does.

use std::io::{BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::process::{Child, Command, ExitCode, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use localflow_core::{Config, Localflow, config_dir};
use localflow_desktop::hotkey::{self, HotkeyController, HotkeyEvent};
use localflow_desktop::learn::{last_dictation, learn_correction, watch};
use localflow_desktop::paths::{self, error, info, make_run_dir, run_dir};

use crate::dictation::{Dictation, learns_on_its_own};
use crate::ui::{self, Start, UiMsg};

const PORT: u16 = 11437;

fn me() -> PathBuf {
    std::env::current_exe().unwrap_or_else(|_| "localflow".into())
}

/// Set up once: the settings window (or `localflow setup`) wrote the env file.
pub fn configured() -> bool {
    config_dir().join("env").exists()
}

/// `localflow` with no arguments.
pub fn app() -> ExitCode {
    if cfg!(target_os = "linux") {
        return settings(); // your key bindings run the dictation; this is where you change things
    }
    if ask_running("settings") {
        return ExitCode::SUCCESS;
    }
    background()
}

/// `localflow settings`: the window, in a process of its own.
pub fn settings() -> ExitCode {
    window(false)
}

/// `localflow onboarding`: the first-run window again.
pub fn onboarding() -> ExitCode {
    window(true)
}

fn window(onboarding: bool) -> ExitCode {
    let _ = paths::init_log();
    let (ui, rx) = ui::channel();
    ui::run(ui, rx, Start { settings: true, quit_with_settings: true, onboarding, then: Box::new(|_, _| {}) });
    ExitCode::SUCCESS
}

/// Tell the running background app to do `what`. False when none runs.
fn ask_running(what: &str) -> bool {
    match TcpStream::connect_timeout(&([127, 0, 0, 1], PORT).into(), Duration::from_millis(300)) {
        Ok(mut s) => s.write_all(format!("{what}\n").as_bytes()).is_ok(),
        Err(_) => false,
    }
}

/// macOS / Windows: wait for the hotkey. The first time (nothing set up
/// yet), the settings window opens; the hotkey works with what it saves.
pub fn background() -> ExitCode {
    let _ = paths::init_log();
    let Ok(lock) = TcpListener::bind(("127.0.0.1", PORT)) else {
        info("already running");
        return ExitCode::SUCCESS;
    };
    if !configured() {
        open_settings();
    }
    std::thread::spawn(move || {
        for stream in lock.incoming().flatten() {
            let mut line = String::new();
            let _ = BufReader::new(stream).read_line(&mut line);
            match line.trim() {
                "settings" => open_settings(),
                "quit" => std::process::exit(0),
                _ => {}
            }
        }
    });
    listen();
    ExitCode::SUCCESS
}

fn open_settings() {
    if let Err(e) = spawn(&["settings"]) {
        error(format_args!("couldn't open the settings: {e}"));
    }
}

/// Show a message in the pill, from a process of its own.
fn show(text: &str, ms: u64) {
    let _ = spawn(&["_message", text, &ms.to_string()]);
}

/// A dictation the hotkey started: its session process.
struct Session(Child);

impl localflow_desktop::hotkey::Dictation for Session {
    fn cancel(mut self) {
        let _ = std::fs::write(run_dir().join(format!("cancel-{}", self.0.id())), "");
        let _ = self.0.wait();
    }
}

/// The hotkey: its listener on one thread, what it does on this one.
fn listen() {
    let (tx, events) = mpsc::channel();
    let started = Instant::now();
    let mut keys = HotkeyController::new(
        || {
            make_run_dir().map_err(|e| e.to_string())?;
            let child = spawn(&["_session"]).map_err(|e| format!("couldn't start: {e}"))?;
            let _ = std::fs::write(run_dir().join("current"), child.id().to_string());
            Ok(Session(child))
        },
        |Session(mut child)| {
            let _ = std::fs::write(run_dir().join(format!("stop-{}", child.id())), "");
            let _ = child.wait(); // presses wait until it has pasted
        },
        |text| show(&text, 3000),
    )
    .with_learn(|| learn_correction(&Localflow::new(Config::from_env())));
    let recording = keys.recording_flag();
    let errors = tx.clone();
    std::thread::spawn(move || {
        if let Err(e) = hotkey::listen(tx, recording) {
            error(format_args!("hotkey: {e}"));
            let _ = errors.send(HotkeyEvent::Error(e));
        }
    });
    for event in events {
        match &event {
            HotkeyEvent::Ready => {
                let (dictate, learn) = hotkey::names();
                info(format_args!("ready: hold {dictate} to dictate, {learn} to learn a correction"));
            }
            HotkeyEvent::Error(e) => {
                show(if e.contains("Accessibility") { "Allow localflow under Accessibility" } else { "Hotkey unavailable, see dictate.log" }, 6000);
            }
            _ => keys.on(&event, started.elapsed().as_secs_f64()),
        }
    }
}

/// `_message TEXT MS`: the pill with a message, then gone.
pub fn run_message(args: &[String]) -> ExitCode {
    let text = args.first().cloned().unwrap_or_default();
    let ms: u64 = args.get(1).and_then(|m| m.parse().ok()).unwrap_or(2500);
    let (ui, rx) = ui::channel();
    ui.send(UiMsg::Message(text, ms));
    ui::run(ui, rx, Start {
        settings: false,
        quit_with_settings: false,
        onboarding: false,
        then: Box::new(move |ui, _| {
            std::thread::spawn(move || {
                std::thread::sleep(Duration::from_millis(ms + 200));
                ui.send(UiMsg::Quit);
            });
        }),
    });
    ExitCode::SUCCESS
}

// --- Linux: one process per dictation ---------------------------------------------

fn session_pid() -> Option<u32> {
    let pid: u32 = std::fs::read_to_string(run_dir().join("current")).ok()?.trim().parse().ok()?;
    alive(pid).then_some(pid)
}

fn alive(pid: u32) -> bool {
    #[cfg(windows)]
    unsafe {
        use windows_sys::Win32::Foundation::{CloseHandle, STILL_ACTIVE};
        use windows_sys::Win32::System::Threading::{GetExitCodeProcess, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION};
        let h = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
        if h.is_null() {
            return false;
        }
        let mut code = 0u32;
        let running = GetExitCodeProcess(h, &mut code) != 0 && code == STILL_ACTIVE as u32;
        CloseHandle(h);
        running
    }
    #[cfg(not(windows))]
    if cfg!(target_os = "linux") {
        PathBuf::from(format!("/proc/{pid}")).exists()
    } else {
        Command::new("kill").args(["-0", &pid.to_string()]).stderr(Stdio::null()).status().is_ok_and(|s| s.success())
    }
}

/// Run `localflow ARGS` in the background, its output in the log.
fn spawn(args: &[&str]) -> std::io::Result<Child> {
    let log = std::fs::OpenOptions::new().create(true).append(true).open({
        let _ = std::fs::create_dir_all(paths::state_dir());
        paths::log_file()
    })?;
    let mut cmd = Command::new(me());
    cmd.args(args).stdin(Stdio::null()).stdout(log.try_clone()?).stderr(log);
    #[cfg(unix)]
    std::os::unix::process::CommandExt::process_group(&mut cmd, 0); // outlives the key binding's shell
    cmd.spawn()
}

pub fn start(raw: bool) -> ExitCode {
    if make_run_dir().is_err() || session_pid().is_some() {
        return ExitCode::SUCCESS;
    }
    match spawn(if raw { &["_session", "raw"] } else { &["_session"] }) {
        Ok(child) => {
            let _ = std::fs::write(run_dir().join("current"), child.id().to_string());
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("localflow: couldn't start: {e}");
            ExitCode::FAILURE
        }
    }
}

/// `stop` or `cancel`: a file the session looks for.
pub fn signal(what: &str) -> ExitCode {
    for _ in 0..20 {
        // the session may still be starting
        if let Some(pid) = session_pid() {
            let _ = std::fs::write(run_dir().join(format!("{what}-{pid}")), "");
            return ExitCode::SUCCESS;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    ExitCode::SUCCESS
}

pub fn toggle(raw: bool) -> ExitCode {
    if session_pid().is_some() { signal("stop") } else { start(raw) }
}

/// The session process `start` spawns: records right away, then shows the
/// pill, and finishes or cancels when `stop` or `cancel` says so.
pub fn run_session(raw: bool) -> ExitCode {
    let (ui, rx) = ui::channel();
    let dictation = match Dictation::start(ui.clone(), raw) {
        Ok(d) => d,
        Err(e) => {
            error(format_args!("couldn't start recording: {e}"));
            localflow_desktop::notify(&format!("localflow: {e}"), 4000);
            release_current();
            return ExitCode::FAILURE;
        }
    };
    let me = std::process::id();
    let (stop, cancel) = (run_dir().join(format!("stop-{me}")), run_dir().join(format!("cancel-{me}")));
    ui::run(ui, rx, Start {
        settings: false,
        quit_with_settings: false,
        onboarding: false,
        then: Box::new(move |ui, _| {
            std::thread::spawn(move || {
                let started = Instant::now();
                loop {
                    if cancel.exists() {
                        let _ = std::fs::remove_file(&cancel);
                        release_current();
                        dictation.cancel();
                        break;
                    }
                    if stop.exists() || started.elapsed() > Duration::from_secs(600) {
                        // (forgotten: stop after 10 minutes)
                        let _ = std::fs::remove_file(&stop);
                        release_current(); // the next dictation can start while this one finishes
                        let ms = dictation.finish();
                        std::thread::sleep(Duration::from_millis(ms + 100)); // let a closing message show
                        break;
                    }
                    std::thread::sleep(Duration::from_millis(30));
                }
                ui.send(UiMsg::Quit);
            });
        }),
    });
    release_current();
    ExitCode::SUCCESS
}

fn release_current() {
    if session_pid() == Some(std::process::id()) {
        let _ = std::fs::remove_file(run_dir().join("current"));
    }
}

/// The learn key: compare what you selected (or copied) with the last
/// dictation, and say what was learned.
pub fn learn() -> ExitCode {
    let _ = paths::init_log();
    let message = learn_correction(&Localflow::new(Config::from_env()));
    let (ui, rx) = ui::channel();
    ui.send(UiMsg::Message(message, 2500));
    ui::run(ui, rx, Start {
        settings: false,
        quit_with_settings: false,
        onboarding: false,
        then: Box::new(|ui, _| {
            std::thread::spawn(move || {
                std::thread::sleep(Duration::from_millis(2700));
                ui.send(UiMsg::Quit);
            });
        }),
    });
    ExitCode::SUCCESS
}

/// Watch for your fixes to what was just pasted, from a short-lived process
/// of its own (the session exits right after pasting). ~25 MB while it does.
pub fn watch_for_fixes(pid: Option<u32>, stamp: f64) {
    if !learns_on_its_own() {
        return;
    }
    if let Err(e) = spawn(&["_watch", &pid.unwrap_or(0).to_string(), &stamp.to_string()]) {
        info(format_args!("couldn't watch for fixes: {e}"));
    }
}

/// `_watch PID STAMP`: the Linux watcher, for the dictation with that stamp.
pub fn run_watch(args: &[String]) -> ExitCode {
    let pid = args.first().and_then(|p| p.parse::<u32>().ok()).filter(|&p| p != 0);
    let stamp: Option<f64> = args.get(1).and_then(|s| s.parse().ok());
    if let (Some((text, latest)), Some(stamp)) = (last_dictation(), stamp)
        && latest == stamp
    {
        let lf = Localflow::new(Config::from_env());
        // What it learned: a notification on Linux (any desktop has them), the pill elsewhere.
        let tell = |m: String| if cfg!(target_os = "linux") { localflow_desktop::notify(&m, 4000) } else { show(&m, 3000) };
        watch(&lf, &text, pid, stamp, &tell);
    }
    ExitCode::SUCCESS
}

/// Key bindings for Hyprland (Omarchy's bindings.lua), running this program.
pub fn linux_bindings() -> String {
    let exe = crate::setup::app_exe().display().to_string();
    let exe = if exe.contains(' ') { format!("'{exe}'") } else { exe };
    [
        "hl.unbind(\"SUPER + CTRL + X\")".to_string(),
        "hl.unbind(\"F9\")".to_string(),
        format!("o.bind(\"SUPER + CTRL + X\", \"Toggle dictation\", \"{exe} toggle\")"),
        format!("o.bind(\"F9\", \"Start dictation (push-to-talk)\", \"{exe} start\")"),
        format!("o.bind(\"F9\", \"Stop dictation (push-to-talk)\", \"{exe} stop\", {{ release = true }})"),
        format!("o.bind(\"F10\", \"Start raw dictation (push-to-talk)\", \"{exe} start raw\")"),
        format!("o.bind(\"F10\", \"Stop raw dictation (push-to-talk)\", \"{exe} stop\", {{ release = true }})"),
        format!("o.bind(\"F8\", \"Learn from a dictation correction\", \"{exe} learn\")"),
    ]
    .join("\n")
}
