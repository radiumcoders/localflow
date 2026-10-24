//! Setting localflow up on this computer: the settings, the speech model,
//! and the app itself where it has to be.
//!
//!     localflow setup                          # cleanup on this computer (downloads its model)
//!     localflow setup --api anthropic          # with your key (LOCALFLOW_API_KEY, or asked for)
//!     localflow setup --api openai --model gpt-5-nano
//!     localflow setup --speech-model base.en
//!
//! The settings window does the same when you press Save. Per OS:
//! Linux: run from the AppImage, it copies itself to ~/.local/share/localflow/app
//! (your key bindings run it from there) and adds itself to the app launcher.
//! macOS: a login item (launchd) starts it in the background.
//! Windows: the .msi starts it at sign-in; setup starts it now.

use std::path::PathBuf;
use std::process::{Command, ExitCode, Stdio};
use std::sync::atomic::AtomicBool;

use localflow_core::{Config, Localflow, Msg, NO_FILES, config_dir, home, prompts::HAIKU, save_api_key, save_settings};
use localflow_speech::models;

const USAGE: &str = "localflow setup [--api PROVIDER|URL] [--model MODEL] [--speech-model ID] [--no-start]";

pub fn run(args: &[String]) -> ExitCode {
    let (mut api, mut model, mut speech, mut start) = (None, None, None, true);
    let mut it = args.iter();
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "--api" => api = it.next().cloned(),
            "--model" => model = it.next().cloned(),
            "--speech-model" => speech = it.next().cloned(),
            "--no-start" => start = false,
            _ => {
                eprintln!("{USAGE}");
                return ExitCode::from(2);
            }
        }
    }
    let dir = config_dir();
    let anthropic = api.as_deref().is_some_and(|a| a.eq_ignore_ascii_case("anthropic"));
    let model = if anthropic { Some(HAIKU.to_string()) } else { model };
    if api.is_some() && model.is_none() {
        eprintln!("localflow: name the provider's model with --model (any but Anthropic, which is always Claude Haiku)");
        return ExitCode::from(2);
    }
    let speech = speech.unwrap_or_else(|| models::default_model().id.to_string());
    let Some(info) = models::MODELS.iter().find(|m| m.id == speech) else {
        eprintln!("localflow: no speech model {speech} (see `localflow models`)");
        return ExitCode::from(2);
    };
    println!("== settings in {}", dir.display());
    let changes = [
        ("LOCALFLOW_API_URL", api.as_deref()),
        ("LOCALFLOW_MODEL", model.as_deref().or(api.is_none().then_some(localflow_core::local::MODEL.id))),
        ("LOCALFLOW_SPEECH_MODEL", Some(info.id)),
    ];
    if let Err(e) = save_settings(&dir, &changes) {
        eprintln!("localflow: couldn't save the settings: {e}");
        return ExitCode::FAILURE;
    }
    if api.is_some()
        && let Ok(key) = std::env::var("LOCALFLOW_API_KEY")
        && let Err(e) = save_api_key(&dir, &key)
    {
        eprintln!("localflow: couldn't save the key: {e}");
        return ExitCode::FAILURE;
    }

    println!("== speech model: {} ({} MB)", info.name, info.size_mb());
    if !models::installed(info.id) {
        let mut last = u64::MAX;
        let mut progress = |done: u64, total: u64| {
            let pct = (done * 100).checked_div(total).unwrap_or(0);
            if pct / 10 != last {
                last = pct / 10;
                println!("   {pct}%");
            }
        };
        if let Err(e) = models::download(info.id, &mut progress, &AtomicBool::new(false)) {
            eprintln!("localflow: couldn't download {}: {e}", info.id);
            return ExitCode::FAILURE;
        }
    }
    println!("   in {}", models::models_dir().display());

    let lf = Localflow::new(Config::from_env());
    println!("== cleanup: {} via {}", lf.cfg.model, lf.cfg.backend());
    if lf.cfg.local() && !lf.cfg.local_model_installed() {
        println!("   downloading {}", crate::cleanup_model::described());
        let mut last = u64::MAX;
        let mut progress = |done: u64, total: u64| {
            let pct = (done * 100).checked_div(total).unwrap_or(0);
            if pct / 10 != last {
                last = pct / 10;
                println!("   {pct}%");
            }
        };
        if let Err(e) = crate::cleanup_model::get_ready(&lf.cfg, &mut progress, &AtomicBool::new(false)) {
            eprintln!("localflow: couldn't download the cleanup model: {e}");
            return ExitCode::FAILURE;
        }
    }
    let answer = if lf.cfg.local() {
        lf.clean("um testing one two three", NO_FILES).map(drop)
    } else {
        lf.chat(&lf.cfg.model.clone(), &[Msg::user("Reply with just: ok")], 32).map(drop)
    };
    match answer {
        Ok(()) => println!("   it answers"),
        Err(e) => {
            eprintln!("localflow: {} doesn't answer: {e}", lf.cfg.model);
            return ExitCode::FAILURE;
        }
    }

    println!("== the app");
    match install_app(start) {
        Ok(done) => println!("   {done}"),
        Err(e) => {
            eprintln!("localflow: {e}");
            return ExitCode::FAILURE;
        }
    }
    if cfg!(target_os = "linux") {
        println!("\nBind keys to dictate (Hyprland: ~/.config/hypr/bindings.lua):\n{}", crate::session::linux_bindings());
    }
    ExitCode::SUCCESS
}

/// Where the Linux app lives once set up (key bindings and the launcher run
/// it from here, so the AppImage can go anywhere, or away).
pub fn linux_app() -> PathBuf {
    let data = std::env::var_os("XDG_DATA_HOME").map(PathBuf::from).unwrap_or_else(|| home().join(".local/share"));
    data.join("localflow/app/localflow")
}

/// The program key bindings and login items should run: the set-up copy on
/// Linux, this one elsewhere.
pub fn app_exe() -> PathBuf {
    let me = std::env::current_exe().unwrap_or_else(|_| "localflow".into());
    if cfg!(target_os = "linux") && (std::env::var_os("APPIMAGE").is_some() || linux_app().exists()) {
        return linux_app();
    }
    me
}

/// Put the app where it runs from (see the module docs). `start`: start the
/// background app now (macOS, Windows).
pub fn install_app(start: bool) -> Result<String, String> {
    let me = std::env::current_exe().map_err(|e| e.to_string())?;
    if cfg!(target_os = "linux") {
        let app = linux_app();
        let dir = app.parent().unwrap();
        std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        if me != app {
            // Copy to a new name, then rename: replacing a running program in place fails.
            let tmp = dir.join("localflow.new");
            std::fs::copy(&me, &tmp).map_err(|e| format!("couldn't copy the app to {}: {e}", app.display()))?;
            std::fs::rename(&tmp, &app).map_err(|e| e.to_string())?;
        }
        let icon = dir.join("localflow.png");
        if let Some(appdir) = std::env::var_os("APPDIR") {
            let _ = std::fs::copy(PathBuf::from(appdir).join("localflow.png"), &icon);
        }
        let apps = app.parent().and_then(|p| p.parent()).and_then(|p| p.parent()).unwrap().join("applications");
        std::fs::create_dir_all(&apps).map_err(|e| e.to_string())?;
        let entry = format!(
            "[Desktop Entry]\nType=Application\nName=localflow\nComment=Talk, and get clean text where you type\n\
             Exec=\"{}\" settings\nIcon={}\nCategories=Utility;\nTerminal=false\n",
            app.display(),
            if icon.exists() { icon.display().to_string() } else { "audio-input-microphone".into() },
        );
        std::fs::write(apps.join("localflow.desktop"), entry).map_err(|e| e.to_string())?;
        return Ok(format!("{}, and in your app launcher", app.display()));
    }
    if cfg!(target_os = "macos") {
        let plist = home().join("Library/LaunchAgents/computer.localflow.dictate.plist");
        let _ = std::fs::create_dir_all(plist.parent().unwrap());
        let text = format!(
            r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
  <key>Label</key><string>computer.localflow.dictate</string>
  <key>ProgramArguments</key><array><string>{}</string><string>--background</string></array>
  <key>RunAtLoad</key><true/>
  <key>KeepAlive</key><dict><key>SuccessfulExit</key><false/></dict>
  <key>ProcessType</key><string>Interactive</string>
</dict></plist>
"#,
            xml_escape(&me.display().to_string())
        );
        std::fs::write(&plist, text).map_err(|e| e.to_string())?;
        if start {
            let domain = format!("gui/{}", uid());
            let quiet = |args: &[&str]| Command::new("launchctl").args(args).stdout(Stdio::null()).stderr(Stdio::null()).status();
            let _ = quiet(&["bootout", &domain, &plist.display().to_string()]);
            std::thread::sleep(std::time::Duration::from_secs(1)); // bootstrap right after bootout can fail
            quiet(&["bootstrap", &domain, &plist.display().to_string()]).map_err(|e| e.to_string())?;
        }
        return Ok("starts at login (allow it under Accessibility and Microphone when macOS asks)".into());
    }
    if start {
        // Windows hands a child every inheritable handle we hold, our own
        // stdout and stderr included: the background app would keep the pipes
        // of whoever ran setup open, and they'd wait for it forever.
        #[cfg(windows)]
        unsafe {
            use windows_sys::Win32::Foundation::{HANDLE_FLAG_INHERIT, SetHandleInformation};
            use windows_sys::Win32::System::Console::{GetStdHandle, STD_ERROR_HANDLE, STD_INPUT_HANDLE, STD_OUTPUT_HANDLE};
            for which in [STD_INPUT_HANDLE, STD_OUTPUT_HANDLE, STD_ERROR_HANDLE] {
                SetHandleInformation(GetStdHandle(which), HANDLE_FLAG_INHERIT, 0);
            }
        }
        let mut cmd = Command::new(&me);
        cmd.arg("--background").stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null());
        #[cfg(windows)]
        std::os::windows::process::CommandExt::creation_flags(&mut cmd, 0x0000_0008 | 0x0000_0200); // DETACHED, new group
        cmd.spawn().map_err(|e| format!("couldn't start the app: {e}"))?;
    }
    Ok("running in the background, and from sign-in".into())
}

fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
}

#[cfg(unix)]
fn uid() -> u32 {
    // `id -u`, without a libc binding for one number.
    Command::new("id").arg("-u").output().ok().and_then(|o| String::from_utf8(o.stdout).ok()).and_then(|s| s.trim().parse().ok()).unwrap_or(501)
}

#[cfg(not(unix))]
fn uid() -> u32 {
    0
}
