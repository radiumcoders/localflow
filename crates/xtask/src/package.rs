//! `cargo xtask package --version X.Y.Z [--features vulkan]`: the release
//! binary, then its installer in build/: an .msi on Windows (WiX), a .dmg on
//! macOS (localflow.app), an AppImage on Linux.
//!
//! Needs cmake (whisper.cpp), WiX on Windows (`dotnet tool install --global
//! wix --version 5.0.2`, plus `wix extension add -g WixToolset.Util.wixext/5.0.2`), and on Linux the headers listed in
//! .github/workflows/package.yml. appimagetool is downloaded.

use std::path::{Path, PathBuf};

use crate::{ROOT, run, run_env};

pub fn main(args: &[String]) -> Result<(), String> {
    let mut version = "0.0.1".to_string();
    let mut features = String::new();
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--version" => version = it.next().cloned().ok_or("--version X.Y.Z")?,
            "--features" => features = it.next().cloned().ok_or("--features LIST")?,
            other => return Err(format!("unknown option {other}")),
        }
    }
    let build = ROOT.join("build");
    std::fs::create_dir_all(&build).map_err(|e| e.to_string())?;
    crate::icon::write(&build)?;
    let exe = cargo(&version, &features)?;
    println!("app: {} ({:.1} MB)", exe.display(), exe.metadata().map_err(|e| e.to_string())?.len() as f64 / 1e6);
    let out = if cfg!(windows) {
        msi(&build, &exe, &version)?
    } else if cfg!(target_os = "macos") {
        dmg(&build, &mac_app(&build, &exe, &version)?, &version)?
    } else {
        appimage(&build, &exe, &version)?
    };
    println!("installer: {}", out.display());
    Ok(())
}

fn cargo(version: &str, features: &str) -> Result<PathBuf, String> {
    let mut args = vec!["build", "--release", "--locked", "-p", "localflow"];
    if !features.is_empty() {
        args.extend(["--features", features]);
    }
    let mut env = vec![("LOCALFLOW_VERSION", version.to_string())];
    if cfg!(target_os = "macos") {
        env.push(("MACOSX_DEPLOYMENT_TARGET", "11.0".into()));
    }
    run_env("cargo", &args, &env)?;
    let target = std::env::var_os("CARGO_TARGET_DIR").map(PathBuf::from).unwrap_or_else(|| ROOT.join("target"));
    Ok(target.join("release").join(if cfg!(windows) { "localflow.exe" } else { "localflow" }))
}

fn s(p: &Path) -> String {
    p.display().to_string()
}

/// localflow.app: the binary, its icon, and Info.plist (no Dock icon: it's a
/// background helper; the microphone message macOS shows when asking).
fn mac_app(build: &Path, exe: &Path, version: &str) -> Result<PathBuf, String> {
    let app = build.join("localflow.app");
    let _ = std::fs::remove_dir_all(&app);
    let io = |e: std::io::Error| e.to_string();
    std::fs::create_dir_all(app.join("Contents/MacOS")).map_err(io)?;
    std::fs::create_dir_all(app.join("Contents/Resources")).map_err(io)?;
    std::fs::copy(exe, app.join("Contents/MacOS/localflow")).map_err(io)?;
    std::fs::copy(build.join("localflow.icns"), app.join("Contents/Resources/localflow.icns")).map_err(io)?;
    let plist = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
  <key>CFBundleName</key><string>localflow</string>
  <key>CFBundleDisplayName</key><string>localflow</string>
  <key>CFBundleExecutable</key><string>localflow</string>
  <key>CFBundleIdentifier</key><string>computer.localflow</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>CFBundleIconFile</key><string>localflow</string>
  <key>CFBundleShortVersionString</key><string>{version}</string>
  <key>CFBundleVersion</key><string>{version}</string>
  <key>LSMinimumSystemVersion</key><string>11.0</string>
  <key>LSUIElement</key><true/>
  <key>NSHighResolutionCapable</key><true/>
  <key>NSMicrophoneUsageDescription</key><string>localflow records what you say while you hold the dictation key, to turn it into text.</string>
</dict></plist>
"#
    );
    std::fs::write(app.join("Contents/Info.plist"), plist).map_err(io)?;
    // Ad-hoc signature (no Developer ID yet: Gatekeeper asks once, see README).
    run("codesign", &["--force", "--deep", "--sign", "-", &s(&app)])?;
    Ok(app)
}

fn dmg(build: &Path, app: &Path, version: &str) -> Result<PathBuf, String> {
    let arch = if cfg!(target_arch = "aarch64") { "arm64" } else { "x64" };
    let out = build.join(format!("localflow-{version}-{arch}.dmg"));
    let folder = build.join("dmg");
    let _ = std::fs::remove_dir_all(&folder);
    std::fs::create_dir_all(&folder).map_err(|e| e.to_string())?;
    run("ditto", &[&s(app), &s(&folder.join("localflow.app"))])?;
    #[cfg(unix)]
    std::os::unix::fs::symlink("/Applications", folder.join("Applications")).map_err(|e| e.to_string())?; // drag the app onto it
    let _ = std::fs::remove_file(&out);
    run("hdiutil", &["create", "-volname", "localflow", "-srcfolder", &s(&folder), "-format", "UDZO", "-ov", &s(&out)])?;
    Ok(out)
}

fn msi(build: &Path, exe: &Path, version: &str) -> Result<PathBuf, String> {
    let app = build.join("app");
    let _ = std::fs::remove_dir_all(&app);
    std::fs::create_dir_all(&app).map_err(|e| e.to_string())?;
    std::fs::copy(exe, app.join("localflow.exe")).map_err(|e| e.to_string())?;
    let out = build.join(format!("localflow-{version}-x64.msi"));
    let wxs = ROOT.join("pkg/localflow.wxs");
    run("wix", &["build", &s(&wxs), "-ext", "WixToolset.Util.wixext", "-arch", "x64", "-d", &format!("Version={version}"), "-d",
                 &format!("AppDir={}", s(&app)), "-d", &format!("Icon={}", s(&build.join("icon.ico"))), "-o", &s(&out)])?;
    Ok(out)
}

#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
const APPIMAGETOOL: &str = "https://github.com/AppImage/appimagetool/releases/download/continuous/appimagetool-{arch}.AppImage";

/// The binary in usr/bin, started by AppRun, with its desktop entry and icon.
/// It uses the system's libraries (audio, Wayland/X11, Vulkan), which every
/// desktop has.
fn appimage(build: &Path, exe: &Path, version: &str) -> Result<PathBuf, String> {
    let arch = if cfg!(target_arch = "aarch64") { "aarch64" } else { "x86_64" };
    let appdir = build.join("AppDir");
    let _ = std::fs::remove_dir_all(&appdir);
    let io = |e: std::io::Error| e.to_string();
    std::fs::create_dir_all(appdir.join("usr/bin")).map_err(io)?;
    std::fs::copy(exe, appdir.join("usr/bin/localflow")).map_err(io)?;
    std::fs::write(appdir.join("AppRun"), "#!/bin/sh\nexec \"$(dirname \"$(readlink -f \"$0\")\")/usr/bin/localflow\" \"$@\"\n").map_err(io)?;
    std::fs::write(
        appdir.join("localflow.desktop"),
        "[Desktop Entry]\nType=Application\nName=localflow\nComment=Talk, and get clean text where you type\n\
         Exec=localflow\nIcon=localflow\nCategories=Utility;\nTerminal=false\n",
    )
    .map_err(io)?;
    std::fs::copy(build.join("icon.png"), appdir.join("localflow.png")).map_err(io)?;
    let tool = build.join(format!("appimagetool-{arch}.AppImage"));
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(appdir.join("AppRun"), std::fs::Permissions::from_mode(0o755)).map_err(io)?;
        std::os::unix::fs::symlink("localflow.png", appdir.join(".DirIcon")).map_err(io)?;
        if !tool.exists() {
            let mut body = ureq::get(&APPIMAGETOOL.replace("{arch}", arch)).call().map_err(|e| e.to_string())?.into_reader();
            let mut file = std::fs::File::create(&tool).map_err(io)?;
            std::io::copy(&mut body, &mut file).map_err(io)?;
            std::fs::set_permissions(&tool, std::fs::Permissions::from_mode(0o755)).map_err(io)?;
        }
    }
    let out = build.join(format!("localflow-{version}-{arch}.AppImage"));
    // Extract-and-run: no FUSE needed to build (CI machines don't have it).
    run_env(&s(&tool), &["--no-appstream", &s(&appdir), &s(&out)], &[("ARCH", arch.into()), ("APPIMAGE_EXTRACT_AND_RUN", "1".into())])?;
    Ok(out)
}
