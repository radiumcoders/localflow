#!/usr/bin/env python3
"""Install localflow on macOS, Windows or Linux. Safe to re-run.

    python3 install.py                     # default model qwen3.5:4b
    python3 install.py --model qwen3.5:2b  # smaller, faster, less accurate

Steps: install Ollama and Handy if missing, pull the model, run the
localflow proxy at login, and point Handy's post-processing at it.
Uses only the Python standard library.
"""

import argparse
import json
import os
import platform
import shutil
import subprocess
import sys
import time
import urllib.request
from pathlib import Path

HERE = Path(__file__).resolve().parent
SCRIPT = HERE / "localflow"
SYSTEM = platform.system()  # "Linux", "Darwin", "Windows"
HOME = Path.home()
OLLAMA_URL = "http://127.0.0.1:11434"
PROXY_URL = "http://127.0.0.1:11435/v1"
PROMPT = "<dictation>${output}</dictation>"  # localflow serve unwraps this

# Linux installs everything per-user (no sudo) under these paths.
LINUX_OLLAMA = HOME / ".local/ollama"
LINUX_HANDY = HOME / ".local/share/handy"
LINUX_BIN = HOME / ".local/bin"


def step(msg):
    print(f"\n==> {msg}", flush=True)


def run(*cmd, check=True):
    print("   $", " ".join(str(c) for c in cmd), flush=True)
    return subprocess.run([str(c) for c in cmd], check=check)


def have(cmd):
    return shutil.which(cmd) is not None


def http_ok(url):
    try:
        urllib.request.urlopen(url, timeout=2).read()
        return True
    except OSError:
        return False


def wait_for(url, seconds=30):
    for _ in range(seconds * 2):
        if http_ok(url):
            return True
        time.sleep(0.5)
    return False


def download(url, dest):
    print(f"   downloading {url}", flush=True)
    with urllib.request.urlopen(url) as resp, open(dest, "wb") as f:
        shutil.copyfileobj(resp, f)


def latest_asset(repo, suffix):
    api = f"https://api.github.com/repos/{repo}/releases/latest"
    release = json.load(urllib.request.urlopen(api))
    for asset in release["assets"]:
        if asset["name"].endswith(suffix):
            return asset["browser_download_url"]
    sys.exit(f"no release asset ending in {suffix} for {repo}")


# --- Ollama ------------------------------------------------------------------

def ollama_bin():
    if SYSTEM == "Linux" and (LINUX_OLLAMA / "bin/ollama").exists():
        return LINUX_OLLAMA / "bin/ollama"
    return shutil.which("ollama")


def install_ollama(rocm):
    step("Ollama")
    ours = SYSTEM == "Linux" and (LINUX_OLLAMA / "bin/ollama").exists()
    if http_ok(f"{OLLAMA_URL}/api/version") and not ours:
        print("   already running")
        return
    if SYSTEM == "Darwin":
        if Path("/Applications/Ollama.app").exists():
            run("open", "-a", "Ollama")  # the app starts the server and keeps it running
        else:
            require("brew", "Install Homebrew (https://brew.sh) or Ollama from https://ollama.com/download")
            if subprocess.run(["brew", "list", "ollama"], capture_output=True).returncode != 0:
                run("brew", "install", "ollama")
            run("brew", "services", "start", "ollama")
    elif SYSTEM == "Windows":
        if not ollama_bin():
            require("winget", "Install Ollama from https://ollama.com/download")
            run("winget", "install", "--id", "Ollama.Ollama", "-e", "--accept-package-agreements",
                "--accept-source-agreements")
        exe = shutil.which("ollama") or Path(os.environ["LOCALAPPDATA"]) / "Programs/Ollama/ollama.exe"
        subprocess.Popen([str(exe), "serve"], creationflags=0x00000008)  # DETACHED_PROCESS
    else:
        install_ollama_linux(rocm)
    if not wait_for(f"{OLLAMA_URL}/api/version"):
        sys.exit("Ollama didn't start; check it, then re-run install.py")


def install_ollama_linux(rocm):
    if not (LINUX_OLLAMA / "bin/ollama").exists():
        if not have("zstd"):
            sys.exit("zstd is needed to unpack Ollama (e.g. `sudo pacman -S zstd` / `sudo apt install zstd`)")
        LINUX_OLLAMA.mkdir(parents=True, exist_ok=True)
        parts = ["ollama-linux-amd64.tar.zst"] + (["ollama-linux-amd64-rocm.tar.zst"] if rocm else [])
        for part in parts:
            url = f"https://github.com/ollama/ollama/releases/latest/download/{part}"
            print(f"   downloading {url}", flush=True)
            run("sh", "-c", f'curl -fsSL "{url}" | zstd -d | tar -x -C "{LINUX_OLLAMA}"')
    LINUX_BIN.mkdir(parents=True, exist_ok=True)
    link = LINUX_BIN / "ollama"
    if link.is_symlink() and not link.exists():
        link.unlink()  # dangling link from an older install
    if not link.exists():
        link.symlink_to(LINUX_OLLAMA / "bin/ollama")
    systemd_unit("ollama", f"""[Unit]
Description=Ollama (user-level, for localflow)
After=network.target

[Service]
ExecStart={LINUX_OLLAMA}/bin/ollama serve
Environment=OLLAMA_HOST=127.0.0.1:11434
Restart=on-failure
RestartSec=3

[Install]
WantedBy=default.target
""")


def has_amd_gpu():
    try:
        out = subprocess.run(["lspci"], capture_output=True, text=True).stdout
    except OSError:
        return False
    return any(("VGA" in l or "3D" in l) and ("AMD" in l or "ATI" in l) for l in out.splitlines())


# --- Handy -------------------------------------------------------------------

def handy_data_dir():
    if SYSTEM == "Darwin":
        return HOME / "Library/Application Support/com.pais.handy"
    if SYSTEM == "Windows":
        return Path(os.environ["APPDATA"]) / "com.pais.handy"
    return Path(os.environ.get("XDG_DATA_HOME", HOME / ".local/share")) / "com.pais.handy"


def handy_launch_cmd():
    if SYSTEM == "Darwin":
        return ["open", "-a", "Handy", "--args", "--start-hidden"]
    if SYSTEM == "Windows":
        for base in (os.environ.get("LOCALAPPDATA"), os.environ.get("ProgramFiles")):
            exe = Path(base or "") / "Handy/Handy.exe"
            if exe.exists():
                return [str(exe), "--start-hidden"]
        return None
    wrapper = LINUX_BIN / "handy"
    if wrapper.exists():
        return [str(wrapper), "--start-hidden"]
    return ["handy", "--start-hidden"] if have("handy") else None


def stop_handy():
    if SYSTEM == "Darwin":
        subprocess.run(["osascript", "-e", 'quit app "Handy"'], capture_output=True)
    elif SYSTEM == "Windows":
        subprocess.run(["taskkill", "/IM", "Handy.exe", "/F"], capture_output=True)
    else:
        subprocess.run(["pkill", "-x", "handy"], capture_output=True)
    time.sleep(1.5)


def start_handy():
    cmd = handy_launch_cmd()
    if not cmd:
        print("   start Handy yourself (couldn't find it to launch)")
        return
    if SYSTEM == "Linux" and have("hyprctl"):
        # Let Hyprland spawn it so Handy gets the desktop session's
        # environment, not this shell's (which may be a terminal sandbox).
        line = " ".join(cmd)
        for dispatch in (f'hl.dsp.exec_cmd("{line}")', f"exec {line}"):  # Lua config, then classic
            r = subprocess.run(["hyprctl", "dispatch", dispatch], capture_output=True, text=True)
            if r.stdout.strip() == "ok":
                return
    flags = 0x00000008 if SYSTEM == "Windows" else 0
    subprocess.Popen(cmd, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL,
                     start_new_session=SYSTEM != "Windows", creationflags=flags)


def install_handy():
    step("Handy")
    if SYSTEM == "Darwin":
        if not Path("/Applications/Handy.app").exists():
            require("brew", "Install Handy from https://handy.computer")
            run("brew", "install", "--cask", "handy")
    elif SYSTEM == "Windows":
        if not handy_launch_cmd():
            require("winget", "Install Handy from https://handy.computer")
            run("winget", "install", "--id", "cjpais.Handy", "-e", "--accept-package-agreements",
                "--accept-source-agreements")
    else:
        install_handy_linux()


def install_handy_linux():
    # AppImage, extracted so each keypress doesn't pay for a FUSE mount.
    appimage, appdir = LINUX_HANDY / "Handy.AppImage", LINUX_HANDY / "app"
    if not appimage.exists():
        LINUX_HANDY.mkdir(parents=True, exist_ok=True)
        download(latest_asset("cjpais/Handy", "_amd64.AppImage"), appimage)
        appimage.chmod(0o755)
    if not appdir.exists():
        stop_handy()
        subprocess.run([str(appimage), "--appimage-extract"], cwd=LINUX_HANDY,
                       stdout=subprocess.DEVNULL, check=True)
        (LINUX_HANDY / "squashfs-root").rename(appdir)
    LINUX_BIN.mkdir(parents=True, exist_ok=True)
    wrapper = LINUX_BIN / "handy"
    wrapper.write_text(f'#!/bin/sh\nexec "{appdir}/AppRun" "$@"\n')
    wrapper.chmod(0o755)
    desktop = HOME / ".local/share/applications/handy.desktop"
    desktop.parent.mkdir(parents=True, exist_ok=True)
    desktop.write_text("[Desktop Entry]\nType=Application\nName=Handy\nComment=Speech to text\n"
                       f"Exec={wrapper}\nIcon={appdir}/handy.png\nCategories=Utility;\n")


def configure_handy(hyprland):
    step("Handy settings")
    store = handy_data_dir() / "settings_store.json"
    if not store.exists():
        # Handy writes its settings on first launch.
        start_handy()
        for _ in range(40):
            if store.exists():
                break
            time.sleep(0.5)
        time.sleep(2)
    if not store.exists():
        sys.exit(f"Handy hasn't created {store} yet: open Handy once, finish setup, then re-run")

    stop_handy()  # Handy overwrites the store with its in-memory copy otherwise
    data = json.loads(store.read_text())
    s = data["settings"]
    s["post_process_enabled"] = True
    s["post_process_provider_id"] = "custom"
    s.setdefault("post_process_models", {})["custom"] = "localflow"
    for p in s.get("post_process_providers", []):
        if p["id"] == "custom":
            p["base_url"] = PROXY_URL
    # "handy_clean" was this project's prompt id before it was renamed.
    prompts = [p for p in s.get("post_process_prompts", []) if p["id"] not in ("localflow", "handy_clean")]
    prompts.append({"id": "localflow", "name": "Clean dictation (local)", "prompt": PROMPT})
    s["post_process_prompts"] = prompts
    s["post_process_selected_prompt_id"] = "localflow"
    # Load the speech model only while dictating: it loads in under a second,
    # in the background as recording starts, and frees its memory right after.
    s["model_unload_timeout"] = "immediately"
    if SYSTEM == "Darwin":
        # The proxy warms the cleanup model when Handy logs that the
        # post-process shortcut was pressed, which Handy logs at debug level.
        s["log_level"] = "debug"
    if hyprland:
        # Hyprland owns the dictation keys (see README). Handy's own global
        # shortcuts grab Ctrl+Space and start stray recordings that block the
        # Hyprland toggles; empty bindings get reset, so park them instead.
        for bid, combo in {"transcribe": "ctrl+alt+shift+f24",
                           "transcribe_with_post_process": "ctrl+alt+shift+f23"}.items():
            if bid in s.get("bindings", {}):
                s["bindings"][bid]["current_binding"] = combo
        # Paste via ./paste: Ctrl+V in apps, Shift+Insert in terminals. Typing
        # would turn list newlines into Enter presses.
        s["paste_method"] = "external_script"
        s["external_script_path"] = str(HERE / "paste")
        s["start_hidden"] = True
    store.write_text(json.dumps(data, indent=2))
    print(f"   patched {store}")
    start_handy()


# --- localflow proxy -------------------------------------------------------

def systemd_unit(name, text):
    unit = HOME / ".config/systemd/user" / f"{name}.service"
    unit.parent.mkdir(parents=True, exist_ok=True)
    if unit.is_symlink():
        unit.unlink()
    unit.write_text(text)
    run("systemctl", "--user", "daemon-reload")
    run("systemctl", "--user", "enable", f"{name}.service")
    run("systemctl", "--user", "restart", f"{name}.service")


def install_proxy(model, hyprland):
    step("localflow proxy (runs at login)")
    py = sys.executable
    # On Hyprland the dictation keys also run `localflow --warm`, which loads
    # the model as you start speaking, so it can unload soon after instead of
    # holding ~1 GB RAM + ~3 GB VRAM for an hour. On macOS the proxy does the
    # same by watching Handy's log for the post-process shortcut.
    keep_alive = "2m" if hyprland or SYSTEM == "Darwin" else "1h"
    if SYSTEM == "Linux":
        systemd_unit("localflow", f"""[Unit]
Description=localflow: Ollama dictation cleanup proxy for Handy
After=ollama.service
Wants=ollama.service

[Service]
ExecStart={py} {SCRIPT} serve
Environment=LOCALFLOW_MODEL={model}
Environment=LOCALFLOW_KEEP_ALIVE={keep_alive}
Restart=on-failure
RestartSec=3

[Install]
WantedBy=default.target
""")
    elif SYSTEM == "Darwin":
        plist = HOME / "Library/LaunchAgents/computer.localflow.plist"
        plist.parent.mkdir(parents=True, exist_ok=True)
        plist.write_text(f"""<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
  <key>Label</key><string>computer.localflow</string>
  <key>ProgramArguments</key><array>
    <string>{py}</string><string>{SCRIPT}</string><string>serve</string>
  </array>
  <key>EnvironmentVariables</key><dict>
    <key>LOCALFLOW_MODEL</key><string>{model}</string>
    <key>LOCALFLOW_KEEP_ALIVE</key><string>{keep_alive}</string>
    <!-- Apple Silicon shares memory between CPU and GPU, so the GPU can read
         the mapped model file directly: 1.3 GB instead of 4.8 GB of app
         memory for qwen3.5:4b, at the same speed. -->
    <key>LOCALFLOW_USE_MMAP</key><string>1</string>
  </dict>
  <key>StandardErrorPath</key><string>{HOME}/Library/Logs/localflow.log</string>
  <key>RunAtLoad</key><true/>
  <key>KeepAlive</key><true/>
</dict></plist>
""")
        domain = f"gui/{os.getuid()}"
        subprocess.run(["launchctl", "bootout", domain, str(plist)], capture_output=True)
        time.sleep(1)  # bootstrap right after bootout can fail with "Input/output error"
        run("launchctl", "bootstrap", domain, plist)
    else:
        # A hidden pythonw process started from the Startup folder.
        pyw = Path(py).with_name("pythonw.exe")
        pyw = pyw if pyw.exists() else Path(py)
        startup = Path(os.environ["APPDATA"]) / "Microsoft/Windows/Start Menu/Programs/Startup"
        (startup / "localflow.cmd").write_text(
            f'@echo off\r\nset LOCALFLOW_MODEL={model}\r\nstart "" "{pyw}" "{SCRIPT}" serve\r\n')
        if http_ok(f"{PROXY_URL}/models"):
            print("   already running; log out and in (or end pythonw.exe) to pick up changes")
        else:
            env = dict(os.environ, LOCALFLOW_MODEL=model)
            subprocess.Popen([str(pyw), str(SCRIPT), "serve"], env=env, creationflags=0x00000008)
    if not wait_for(f"{PROXY_URL}/models", 15):
        sys.exit("localflow proxy didn't start; try `python3 localflow serve` to see why")


def require(cmd, hint):
    if not have(cmd):
        sys.exit(f"`{cmd}` not found. {hint}, then re-run install.py")


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--model", default="qwen3.5:4b", help="Ollama model for cleanup (default: qwen3.5:4b)")
    ap.add_argument("--rocm", action=argparse.BooleanOptionalAction, default=None,
                    help="Linux: also install Ollama's ROCm build (default: auto-detect AMD GPU)")
    args = ap.parse_args()
    rocm = has_amd_gpu() if args.rocm is None else args.rocm
    hyprland = SYSTEM == "Linux" and have("hyprctl") and bool(os.environ.get("HYPRLAND_INSTANCE_SIGNATURE"))

    install_ollama(rocm)
    step(f"Model {args.model}")
    run(ollama_bin() or "ollama", "pull", args.model)
    install_proxy(args.model, hyprland)
    install_handy()
    configure_handy(hyprland)

    step("Done")
    if SYSTEM == "Darwin":
        print("   Dictate with Option+Shift+Space (Handy's post-process shortcut).")
    elif SYSTEM == "Windows":
        print("   Dictate with Ctrl+Shift+Space (Handy's post-process shortcut).")
    elif hyprland:
        print("   Bind your dictation keys to `handy --toggle-post-process & localflow --warm` (see README).")
    else:
        print("   Add a desktop shortcut running `handy --toggle-post-process` (see README).")
    print(f"   Test the model: echo \"let's meet at 3 wait at 4\" | {SCRIPT}")


if __name__ == "__main__":
    main()
