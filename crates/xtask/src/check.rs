//! End-to-end checks of the real app on a real desktop (CI machines too):
//!
//!     cargo xtask e2e                      # the release build in target/
//!     cargo xtask installed INSTALLER      # .msi / .dmg / .AppImage: install, check, remove
//!
//! Against a fake cleanup API, with the small.en speech model:
//! 1. setup without the window (`localflow setup`)
//! 2. (installers) the app where it belongs: Program Files and the Start
//!    menu, /Applications and its login item, ~/.local/share and the launcher
//! 3. (macOS / Windows) the background app setup started, hotkey ready
//! 4. transcription: a sentence from the OS's text-to-speech, through our
//!    speech engine and the cleanup
//! 5. dictation into a text box in another process (localflow-testbox): the
//!    hotkey (injected key presses) or `localflow start` / `stop` (Linux),
//!    the overlay, transcription, cleanup, paste. Only the microphone is
//!    replaced, by the WAV (LOCALFLOW_TEST_WAV).
//! 6. `localflow --selftest`
//!
//! Prints PASS / FAIL per step; fails if any step did.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use crate::{ROOT, run, run_env};

const SENTENCE: &str = "Please call mom tomorrow at five.";
/// What the sentence has to come out as, at least.
const HEARD: &str = "call mom tomorrow";
/// The smallest that gets synthetic voices right (tiny.en heard "to mama",
/// base.en "Montemorra" for "mom tomorrow").
const SPEECH_MODEL: &str = "small.en";

struct Steps(Vec<String>);

impl Steps {
    fn step(&mut self, name: &str, f: impl FnOnce() -> Result<String, String>) -> bool {
        println!("\n--- {name}");
        match f() {
            Ok(detail) => {
                println!("PASS {name}{}", if detail.is_empty() { String::new() } else { format!(": {detail}") });
                true
            }
            Err(e) => {
                println!("FAIL {name}: {e}");
                self.0.push(name.to_string());
                false
            }
        }
    }

    fn finish(self) -> Result<(), String> {
        if self.0.is_empty() {
            println!("\nall passed");
            Ok(())
        } else {
            Err(format!("{} failed: {:?}", self.0.len(), self.0))
        }
    }
}

fn s(p: &Path) -> String {
    p.display().to_string()
}

fn home() -> PathBuf {
    PathBuf::from(std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" }).unwrap_or_default())
}

fn env_dir(name: &str, default: impl FnOnce() -> PathBuf) -> PathBuf {
    std::env::var_os(name).filter(|v| !v.is_empty()).map(PathBuf::from).unwrap_or_else(default)
}

/// dictate.log, where the app says what it did (never what you said).
fn log_file() -> PathBuf {
    let dir = if cfg!(windows) {
        env_dir("LOCALAPPDATA", home).join("localflow")
    } else if cfg!(target_os = "macos") {
        home().join("Library/Logs/localflow")
    } else {
        env_dir("XDG_STATE_HOME", || home().join(".local/state")).join("localflow")
    };
    dir.join("dictate.log")
}

fn log_text() -> String {
    std::fs::read_to_string(log_file()).unwrap_or_default()
}

fn temp() -> PathBuf {
    let dir = std::env::temp_dir().join(format!("localflow-e2e-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// Run the app, show its output, fail on a non-zero exit.
fn app(exe: &Path, args: &[&str], env: &[(&str, String)]) -> Result<String, String> {
    let out = Command::new(exe)
        .args(args)
        .envs(env.iter().map(|(k, v)| (*k, v.as_str())))
        .stdin(Stdio::null())
        .output()
        .map_err(|e| format!("{}: {e}", exe.display()))?;
    let (stdout, stderr) = (String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
    let tail = |t: &str| t.chars().rev().take(3000).collect::<Vec<_>>().into_iter().rev().collect::<String>();
    println!("{}{}", tail(&stdout), tail(&stderr));
    if !out.status.success() {
        return Err(format!("localflow {} exited {}", args.join(" "), out.status));
    }
    Ok(stdout.into_owned())
}

/// The sentence as 16 kHz mono 16-bit speech, from the OS's text-to-speech.
fn speak(dir: &Path) -> Result<(PathBuf, f32), String> {
    let wav = dir.join("speech.wav");
    if cfg!(target_os = "macos") {
        let aiff = dir.join("speech.aiff");
        run("say", &["-o", &s(&aiff), SENTENCE])?;
        run("afconvert", &["-f", "WAVE", "-d", "LEI16@16000", "-c", "1", &s(&aiff), &s(&wav)])?;
    } else if cfg!(windows) {
        let ps = format!(
            "Add-Type -AssemblyName System.Speech; $s = New-Object System.Speech.Synthesis.SpeechSynthesizer; \
             $f = New-Object System.Speech.AudioFormat.SpeechAudioFormatInfo(16000, \
             [System.Speech.AudioFormat.AudioBitsPerSample]::Sixteen, [System.Speech.AudioFormat.AudioChannel]::Mono); \
             $s.SetOutputToWaveFile('{}', $f); $s.Speak('{SENTENCE}'); $s.Dispose()",
            s(&wav)
        );
        run("powershell", &["-NoProfile", "-Command", &ps])?;
    } else {
        let raw = dir.join("raw.wav");
        run("espeak-ng", &["-w", &s(&raw), SENTENCE])?;
        run("ffmpeg", &["-loglevel", "error", "-y", "-i", &s(&raw), "-ar", "16000", "-ac", "1", "-sample_fmt", "s16", &s(&wav)])?;
    }
    let bytes = std::fs::read(&wav).map_err(|e| e.to_string())?;
    let seconds = bytes.len().saturating_sub(44) as f32 / 32000.0;
    Ok((wav, seconds))
}

fn setup(exe: &Path, api: &str, start: bool) -> Result<String, String> {
    let mut args = vec!["setup", "--api", api, "--model", "fake-small", "--speech-model", SPEECH_MODEL];
    if !start {
        args.push("--no-start");
    }
    let out = app(exe, &args, &[("LOCALFLOW_API_KEY", "test-key".into())])?;
    Ok(out.lines().rfind(|l| !l.trim().is_empty()).unwrap_or("set up").trim().to_string())
}

fn transcribe(exe: &Path, wav: &Path) -> Result<String, String> {
    let out = app(exe, &["transcribe", &s(wav)], &[])?;
    if !out.to_lowercase().contains(HEARD) {
        return Err(format!("heard {out:?}"));
    }
    Ok(out.trim().to_string())
}

/// The background app holds this port while it runs.
fn running() -> bool {
    std::net::TcpListener::bind(("127.0.0.1", 11437)).is_err()
}

fn wait_for(seconds: u64, done: impl Fn(&str) -> bool) -> String {
    let start = Instant::now();
    loop {
        let text = log_text();
        if done(&text) || start.elapsed() > Duration::from_secs(seconds) {
            return text;
        }
        std::thread::sleep(Duration::from_millis(500));
    }
}

fn ready(text: &str) -> bool {
    text.contains("ready: hold") || text.contains("Accessibility")
}

fn tail(text: &str, n: usize) -> String {
    text.chars().rev().take(n).collect::<Vec<_>>().into_iter().rev().collect()
}

fn background(stop: &dyn Fn()) -> Result<String, String> {
    let text = wait_for(60, ready);
    let up = running();
    stop();
    if !up {
        return Err(format!("not running; log:\n{}", tail(&text, 1500)));
    }
    if text.contains("ready: hold") {
        return Ok("running, hotkey ready (stopped it for the next steps)".into());
    }
    if cfg!(target_os = "macos") && text.contains("Accessibility") {
        return Ok("running; the hotkey waits for the Accessibility permission (as on a new Mac)".into());
    }
    Err(format!("running but the hotkey isn't ready; log:\n{}", tail(&text, 1500)))
}

fn stop_background() {
    if cfg!(windows) {
        let _ = Command::new("taskkill").args(["/IM", "localflow.exe", "/F"]).output();
    } else if cfg!(target_os = "macos") {
        let uid = Command::new("id").arg("-u").output().map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string()).unwrap_or_default();
        let _ = Command::new("launchctl").args(["bootout", &format!("gui/{uid}/computer.localflow.dictate")]).output();
    }
    std::thread::sleep(Duration::from_secs(1));
}

/// Dictate the spoken sentence into a text box in another process.
fn dictate(exe: &Path, testbox: &Path, wav: &Path, seconds: f32, dir: &Path) -> Result<String, String> {
    let (out, flag) = (dir.join("typed.txt"), dir.join("pasted"));
    let _ = std::fs::remove_file(&out);
    let _ = std::fs::remove_file(&flag);
    let env = [("LOCALFLOW_TEST_WAV", s(wav)), ("LOCALFLOW_LEARN", "off".into()), ("DICTATE_TEST_INJECTED", "1".into())];
    let mut text_box = Command::new(testbox).args([s(&out), s(&flag)]).spawn().map_err(|e| format!("{}: {e}", testbox.display()))?;
    let mut background = None;
    let result = (|| {
        std::thread::sleep(Duration::from_millis(2500)); // let the text box take focus
        let start = log_text().len();
        let held = Duration::from_secs_f32(seconds + 0.8);
        if cfg!(target_os = "linux") {
            run_env(&s(exe), &["start"], &env)?;
            std::thread::sleep(held);
            run_env(&s(exe), &["stop"], &env)?;
        } else {
            background = Some(Command::new(exe).arg("--background").envs(env.iter().map(|(k, v)| (*k, v.as_str())))
                .spawn().map_err(|e| e.to_string())?);
            let text = wait_for(60, |t| ready(&t[start.min(t.len())..]));
            if !text[start.min(text.len())..].contains("ready: hold") {
                return Err(format!("the hotkey isn't ready; log:\n{}", tail(&text[start.min(text.len())..], 1500)));
            }
            crate::keys::hotkey(true);
            std::thread::sleep(held); // held: hold to talk
            crate::keys::hotkey(false);
        }
        let text = wait_for(90, |t| {
            let new = &t[start.min(t.len())..];
            new.contains("done in") || new.contains("failed") || new.contains("nothing heard")
        });
        std::thread::sleep(Duration::from_millis(1500));
        let _ = std::fs::write(&flag, "");
        let deadline = Instant::now() + Duration::from_secs(30);
        while !out.exists() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(200));
        }
        let got = std::fs::read_to_string(&out).unwrap_or_default();
        println!("{}", tail(&text[start.min(text.len())..], 2000));
        if !got.to_lowercase().contains(HEARD) {
            return Err(format!("the text box got {got:?}"));
        }
        Ok(format!("pasted {got:?}"))
    })();
    let _ = text_box.kill();
    if let Some(mut b) = background {
        let _ = b.kill();
    }
    result
}

fn selftest(exe: &Path) -> Result<String, String> {
    let out = app(exe, &["--selftest", "3"], &[])?;
    if !out.contains("\n0 failed") {
        return Err("a check failed".into());
    }
    Ok("all checks passed".into())
}

fn built(name: &str) -> PathBuf {
    let target = std::env::var_os("CARGO_TARGET_DIR").map(PathBuf::from).unwrap_or_else(|| ROOT.join("target"));
    target.join("release").join(if cfg!(windows) { format!("{name}.exe") } else { name.to_string() })
}

/// The checks every installed (or built) app gets: steps 4-6.
fn common(steps: &mut Steps, exe: &Path, dir: &Path) {
    let testbox = built("localflow-testbox");
    let mut speech = None;
    steps.step("text-to-speech WAV", || {
        let (wav, secs) = speak(dir)?;
        speech = Some((wav, secs));
        Ok(format!("{secs:.1}s of speech"))
    });
    let Some((wav, seconds)) = speech else { return };
    steps.step("transcription and cleanup", || transcribe(exe, &wav));
    steps.step("dictation into a text box", || dictate(exe, &testbox, &wav, seconds, dir));
    steps.step("self-test", || selftest(exe));
}

/// `cargo xtask e2e`: the release build (and the test text box), as built.
pub fn e2e(_: &[String]) -> Result<(), String> {
    run("cargo", &["build", "--release", "--locked", "-p", "localflow", "-p", "localflow-testbox"])?;
    let exe = built("localflow");
    let api = crate::fake_api::start();
    let dir = temp();
    let mut steps = Steps(vec![]);
    if steps.step("setup", || setup(&exe, &api, false)) {
        common(&mut steps, &exe, &dir);
    }
    steps.finish()
}

/// `cargo xtask installed INSTALLER`.
pub fn installed(args: &[String]) -> Result<(), String> {
    let installer = PathBuf::from(args.first().ok_or("cargo xtask installed INSTALLER")?);
    let installer = crate::absolute(&installer).map_err(|e| format!("{}: {e}", installer.display()))?;
    run("cargo", &["build", "--release", "--locked", "-p", "localflow-testbox"])?;
    let api = crate::fake_api::start();
    let dir = temp();
    let mut steps = Steps(vec![]);
    if cfg!(windows) {
        windows(&mut steps, &installer, &api, &dir);
    } else if cfg!(target_os = "macos") {
        macos(&mut steps, &installer, &api, &dir);
    } else {
        linux(&mut steps, &installer, &api, &dir);
    }
    steps.finish()
}

fn windows(steps: &mut Steps, msi: &Path, api: &str, dir: &Path) {
    let program_files = env_dir("ProgramFiles", || "C:\\Program Files".into());
    let exe = program_files.join("localflow").join("localflow.exe"); // backslashes, as the Run entry has it
    let shortcut = env_dir("ProgramData", || "C:\\ProgramData".into()).join("Microsoft/Windows/Start Menu/Programs/localflow.lnk");
    let run_entry = || {
        Command::new("reg").args(["query", r"HKCU\Software\Microsoft\Windows\CurrentVersion\Run", "/v", "localflow"]).output()
            .ok().filter(|o| o.status.success()).map(|o| String::from_utf8_lossy(&o.stdout).to_string())
    };
    let msiexec = |args: &[&str]| -> Result<(), String> {
        let log = dir.join("msiexec.log");
        let st = Command::new("msiexec").args(args).args(["/qn", "/l*v", &s(&log)]).status().map_err(|e| e.to_string())?;
        if !st.success() {
            return Err(format!("msiexec {} exited {st}", args[0]));
        }
        Ok(())
    };
    let ok = steps.step("install the .msi", || {
        msiexec(&["/i", &s(msi)])?;
        if !exe.exists() || !shortcut.exists() {
            return Err(format!("{} exists: {}, Start menu shortcut: {}", exe.display(), exe.exists(), shortcut.exists()));
        }
        let entry = run_entry().ok_or("no Run entry")?;
        if !entry.to_lowercase().contains(&s(&exe).to_lowercase()) {
            return Err(format!("Run entry: {entry}"));
        }
        Ok(format!("{}, Start menu shortcut, starts at sign-in", exe.display()))
    });
    if ok {
        if steps.step("setup in the app", || setup(&exe, api, true)) {
            steps.step("background app", || background(&stop_background));
        }
        common(steps, &exe, dir);
    }
    steps.step("uninstall", || {
        stop_background();
        msiexec(&["/x", &s(msi)])?;
        for _ in 0..10 {
            if !exe.exists() {
                break;
            }
            std::thread::sleep(Duration::from_secs(1)); // Program Files can lag behind msiexec
        }
        if exe.exists() || shortcut.exists() || run_entry().is_some() {
            return Err("app, shortcut or Run entry still there".into());
        }
        Ok("app, shortcut and Run entry removed".into())
    });
}

fn macos(steps: &mut Steps, dmg: &Path, api: &str, dir: &Path) {
    let app = PathBuf::from("/Applications/localflow.app");
    let exe = app.join("Contents/MacOS/localflow");
    let agent = home().join("Library/LaunchAgents/computer.localflow.dictate.plist");
    let ok = steps.step("install from the .dmg", || {
        let mount = dir.join("mount");
        std::fs::create_dir_all(&mount).map_err(|e| e.to_string())?;
        run("hdiutil", &["attach", "-nobrowse", "-quiet", "-mountpoint", &s(&mount), &s(dmg)])?;
        let copied = (|| {
            if !mount.join("localflow.app").is_dir() {
                return Err("no localflow.app in the .dmg".to_string());
            }
            if std::fs::read_link(mount.join("Applications")).ok() != Some(PathBuf::from("/Applications")) {
                return Err("no Applications shortcut".into());
            }
            let _ = std::fs::remove_dir_all(&app);
            run("ditto", &[&s(&mount.join("localflow.app")), &s(&app)])
        })();
        let _ = run("hdiutil", &["detach", "-quiet", &s(&mount)]);
        copied?;
        run("codesign", &["--verify", "--deep", "--strict", &s(&app)])?;
        let info = std::fs::read_to_string(app.join("Contents/Info.plist")).map_err(|e| e.to_string())?;
        if !info.contains("NSMicrophoneUsageDescription") {
            return Err("no microphone message (macOS would kill it on recording)".into());
        }
        Ok(format!("{}, signed, with its microphone message", app.display()))
    });
    if !ok {
        return;
    }
    if steps.step("setup in the app", || {
        let done = setup(&exe, api, true)?;
        let plist = std::fs::read_to_string(&agent).map_err(|e| format!("no login item: {e}"))?;
        if !plist.contains(&format!("<string>{}</string><string>--background</string>", exe.display())) {
            return Err(format!("login item runs something else:\n{plist}"));
        }
        Ok(format!("{done}; login item runs {} --background", exe.display()))
    }) {
        steps.step("background app at login", || background(&stop_background));
    }
    common(steps, &exe, dir);
    steps.step("remove", || {
        stop_background();
        let _ = std::fs::remove_file(&agent);
        std::fs::remove_dir_all(&app).map_err(|e| e.to_string())?;
        Ok("app and login item removed".into())
    });
}

fn linux(steps: &mut Steps, appimage: &Path, api: &str, dir: &Path) {
    let data = env_dir("XDG_DATA_HOME", || home().join(".local/share"));
    let exe = data.join("localflow/app/localflow");
    let entry = data.join("applications/localflow.desktop");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(appimage, std::fs::Permissions::from_mode(0o755));
    }
    // CI machines have no FUSE; users' do (the AppImage mounts itself).
    let ok = steps.step("setup in the AppImage", || {
        let out = app(appimage, &["setup", "--api", api, "--model", "fake-small", "--speech-model", SPEECH_MODEL],
                      &[("LOCALFLOW_API_KEY", "test-key".into()), ("APPIMAGE_EXTRACT_AND_RUN", "1".into())])?;
        Ok(out.lines().filter(|l| l.starts_with("   ")).collect::<Vec<_>>().join(";"))
    });
    if !ok {
        return;
    }
    steps.step("installed app and launcher entry", || {
        let text = std::fs::read_to_string(&entry).map_err(|e| format!("{}: {e}", entry.display()))?;
        if !exe.exists() || !text.contains(&format!("Exec=\"{}\"", exe.display())) {
            return Err(format!("{} exists: {}; entry:\n{text}", exe.display(), exe.exists()));
        }
        Ok(format!("{}, launcher entry", exe.display()))
    });
    steps.step("runs without the AppImage", || {
        let away = appimage.with_extension("away");
        std::fs::rename(appimage, &away).map_err(|e| e.to_string())?;
        let out = app(&exe, &["--check"], &[]);
        std::fs::rename(&away, appimage).map_err(|e| e.to_string())?;
        Ok(out?.trim().to_string())
    });
    common(steps, &exe, dir);
}
