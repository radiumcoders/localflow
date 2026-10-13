#!/usr/bin/env python3
"""Install localflow on macOS, Windows or Linux. Safe to re-run.

    python3 install.py                     # qwen3.5:4b (MLX build on Apple Silicon)
    python3 install.py --model qwen3.5:2b  # smaller, faster, less accurate
    python3 install.py --api anthropic     # Claude Haiku with your own API key, no Ollama
    python3 install.py --api openai --model <model>  # any other provider: name its model

Steps: install Ollama (or check your API key) and Handy, pull the model, and
set up `dictate`: a bottom-of-screen pill that records while you hold the
hotkey, transcribes with Handy's speech engine, cleans up and pastes. On
Windows and macOS it starts at login (hotkey Ctrl+Shift+Space / Option+Shift+
Space); on Linux you bind keys to `dictate start` / `dictate stop`.
--handy-app sets up the older way instead: Handy's own app and hotkeys, with
the localflow proxy as its post-processing step.
With --api, skip Ollama and send the cleanup to that provider instead; the key
comes from LOCALFLOW_API_KEY, the key saved by an earlier run, or a prompt.
Uses only the Python standard library.
"""

import argparse
import getpass
import json
import os
import platform
import shutil
import subprocess
import sys
import tempfile
import time
import urllib.request
from pathlib import Path

HERE = Path(__file__).resolve().parent
SCRIPT = HERE / "localflow"
DICTATE = HERE / "dictate"
SYSTEM = platform.system()  # "Linux", "Darwin", "Windows"
HOME = Path.home()
OLLAMA_URL = "http://127.0.0.1:11434"
PROXY_URL = "http://127.0.0.1:11435/v1"
PROMPT = "<dictation>${output}</dictation>"  # localflow serve unwraps this

# Linux installs everything per-user (no sudo) under these paths.
LINUX_OLLAMA = HOME / ".local/ollama"
LINUX_HANDY = HOME / ".local/share/handy"
LINUX_BIN = HOME / ".local/bin"

# Where localflow reads the API key (see LOCALFLOW_API_URL in localflow).
# On Anthropic, cleanup always runs on Claude Haiku 5.5 (HAIKU in localflow).
HAIKU = "claude-haiku-5-5"

KEY_FILE = (Path(os.environ["APPDATA"]) if SYSTEM == "Windows" else
            Path(os.environ.get("XDG_CONFIG_HOME") or HOME / ".config")) / "localflow/api_key"


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


def model_works(model):
    """One tiny request with the same load options localflow uses."""
    body = {"model": model, "messages": [{"role": "user", "content": "Say ok."}], "stream": False,
            "think": False, "options": {"num_predict": 4, "use_mmap": False}}
    try:
        req = urllib.request.Request(f"{OLLAMA_URL}/api/chat", data=json.dumps(body).encode(),
                                     headers={"Content-Type": "application/json"})
        urllib.request.urlopen(req, timeout=300).read()
        return True
    except OSError as e:
        print(f"   {model} didn't run: {e}")
        return False


def pull_model(requested):
    """Pull the cleanup model. On Apple Silicon, prefer Ollama's MLX build
    (faster there) and fall back to the regular one if it can't run."""
    if requested:
        candidates = [requested]
    elif SYSTEM == "Darwin" and platform.machine() == "arm64":
        candidates = ["qwen3.5:4b-mlx", "qwen3.5:4b"]
    else:
        candidates = ["qwen3.5:4b"]
    for model in candidates:
        step(f"Model {model}")
        if run(ollama_bin() or "ollama", "pull", model, check=False).returncode == 0 and model_works(model):
            return model
    sys.exit(f"couldn't pull and run {' or '.join(candidates)}; check Ollama, then re-run install.py")


def has_amd_gpu():
    try:
        out = subprocess.run(["lspci"], capture_output=True, text=True).stdout
    except OSError:
        return False
    return any(("VGA" in l or "3D" in l) and ("AMD" in l or "ATI" in l) for l in out.splitlines())


def setup_api(api, model):
    """Save the API key where only you can read it, then check that the model answers."""
    step(f"API {api} ({model})")
    key = os.environ.get("LOCALFLOW_API_KEY", "").strip()
    if not key and KEY_FILE.exists():
        print(f"   using the key saved in {KEY_FILE}", flush=True)
    else:
        key = key or getpass.getpass("   API key (not shown): ").strip()
        if not key:
            sys.exit("no API key given")
        KEY_FILE.parent.mkdir(parents=True, exist_ok=True)
        if KEY_FILE.exists():
            KEY_FILE.unlink()
        fd = os.open(KEY_FILE, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
        with os.fdopen(fd, "w") as f:
            f.write(key + "\n")
        print(f"   saved the key to {KEY_FILE}", flush=True)
    env = dict(os.environ, LOCALFLOW_API_URL=api, LOCALFLOW_MODEL=model)
    env.pop("LOCALFLOW_API_KEY", None)  # check the saved key, which the proxy will use
    if subprocess.run([sys.executable, str(SCRIPT), "--check"], env=env).returncode != 0:
        sys.exit(f"the API check failed (see above); fix the provider or model, or the key (delete {KEY_FILE} "
                 "to be asked again), then re-run")


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


ARM = platform.machine().lower() in ("arm64", "aarch64")


def install_handy():
    step("Handy")
    if SYSTEM == "Darwin":
        if not mac_handy_app():
            if have("brew"):
                run("brew", "install", "--cask", "handy")
            else:
                install_handy_dmg()
    elif SYSTEM == "Windows":
        if not handy_launch_cmd():
            if have("winget"):
                run("winget", "install", "--id", "cjpais.Handy", "-e", "--accept-package-agreements",
                    "--accept-source-agreements")
            if not handy_launch_cmd():  # no winget, or it didn't take: the installer from GitHub
                setup = Path(os.environ["TEMP"]) / "Handy-setup.exe"
                download(latest_asset("cjpais/Handy", "_arm64-setup.exe" if ARM else "_x64-setup.exe"), setup)
                run(setup, "/S")  # silent, per-user (NSIS)
            if not handy_launch_cmd():
                sys.exit("Handy didn't install; install it from https://handy.computer, then re-run")
    else:
        install_handy_linux()


def mac_handy_app():
    for app in (Path("/Applications/Handy.app"), HOME / "Applications/Handy.app"):
        if app.exists():
            return app
    return None


def install_handy_dmg():
    """Without Homebrew: the .dmg from GitHub, copied to /Applications (or ~/Applications)."""
    dmg = Path(tempfile.gettempdir()) / "Handy.dmg"
    download(latest_asset("cjpais/Handy", "_aarch64.dmg" if ARM else "_x64.dmg"), dmg)
    mount = Path(tempfile.mkdtemp())
    run("hdiutil", "attach", "-nobrowse", "-quiet", "-mountpoint", mount, dmg)
    try:
        for dest in (Path("/Applications"), HOME / "Applications"):
            dest.mkdir(exist_ok=True)
            if subprocess.run(["cp", "-R", str(mount / "Handy.app"), str(dest)]).returncode == 0:
                break
    finally:
        subprocess.run(["hdiutil", "detach", "-quiet", str(mount)])
    if not mac_handy_app():
        sys.exit("Handy didn't install; install it from https://handy.computer, then re-run")


def install_handy_linux():
    # AppImage, extracted so each keypress doesn't pay for a FUSE mount.
    appimage, appdir = LINUX_HANDY / "Handy.AppImage", LINUX_HANDY / "app"
    if not appimage.exists():
        LINUX_HANDY.mkdir(parents=True, exist_ok=True)
        download(latest_asset("cjpais/Handy", "_aarch64.AppImage" if ARM else "_amd64.AppImage"), appimage)
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


def cleanup_env(model, api, keep_alive=None):
    env = {"LOCALFLOW_MODEL": model, **({"LOCALFLOW_API_URL": api} if api else {})}
    if keep_alive and not api:
        env["LOCALFLOW_KEEP_ALIVE"] = keep_alive
    return env


def write_env(env):
    """Settings for `dictate` and the stdin filter, which don't run under a service."""
    KEY_FILE.parent.mkdir(parents=True, exist_ok=True)
    (KEY_FILE.parent / "env").write_text("".join(f"{k}={v}\n" for k, v in env.items()))


def install_proxy(model, hyprland, api):
    step("localflow proxy (runs at login)")
    py = sys.executable
    env = {"LOCALFLOW_MODEL": model, **({"LOCALFLOW_API_URL": api} if api else {})}
    # On Hyprland the dictation keys also run `localflow --warm`, which loads
    # the model as you start speaking, so it can unload soon after instead of
    # holding ~1 GB RAM + ~3 GB VRAM for an hour.
    if hyprland and not api:
        env["LOCALFLOW_KEEP_ALIVE"] = "2m"
    write_env(env)
    if SYSTEM == "Linux":
        after = "" if api else "After=ollama.service\nWants=ollama.service\n"
        lines = "".join(f"Environment={k}={v}\n" for k, v in env.items())
        systemd_unit("localflow", f"""[Unit]
Description=localflow: dictation cleanup proxy for Handy
After=network-online.target
{after}
[Service]
ExecStart={py} {SCRIPT} serve
{lines}Restart=on-failure
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
{"".join(f"    <key>{k}</key><string>{v}</string>\n" for k, v in env.items())}  </dict>
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
            "@echo off\r\n" + "".join(f"set {k}={v}\r\n" for k, v in env.items())
            + f'start "" "{pyw}" "{SCRIPT}" serve\r\n')
        if http_ok(f"{PROXY_URL}/models"):
            print("   already running; log out and in (or end pythonw.exe) to pick up changes")
        else:
            subprocess.Popen([str(pyw), str(SCRIPT), "serve"], env=dict(os.environ, **env),
                             creationflags=0x00000008)
    if not wait_for(f"{PROXY_URL}/models", 15):
        sys.exit("localflow proxy didn't start; try `python3 localflow serve` to see why")


# --- dictate --------------------------------------------------------------------

def handy_cli():
    """Handy's executable, for its headless --transcribe-file / --list-models."""
    if SYSTEM == "Darwin":
        import plistlib
        app = mac_handy_app() or Path("/Applications/Handy.app")
        try:
            with open(app / "Contents/Info.plist", "rb") as f:
                return app / "Contents/MacOS" / plistlib.load(f)["CFBundleExecutable"]
        except (OSError, KeyError, ValueError):
            return None
    cmd = handy_launch_cmd()
    return Path(cmd[0]) if cmd else None


def handy_models():
    """Handy's model catalog (with is_downloaded), from `handy --list-models --json`."""
    exe = handy_cli()
    if not exe:
        return []
    try:
        r = subprocess.run([str(exe), "--list-models", "--json"], capture_output=True, timeout=60,
                           creationflags=0x08000000 if SYSTEM == "Windows" else 0)
        out = r.stdout
    except subprocess.TimeoutExpired as e:  # it can linger after printing
        out = e.stdout or b""
    try:
        return json.loads(out)
    except (ValueError, TypeError):
        return []


def hf_cache():
    """Where Handy (like other Hugging Face tools) keeps downloaded models."""
    if os.environ.get("HF_HUB_CACHE"):
        return Path(os.environ["HF_HUB_CACHE"])
    return Path(os.environ.get("HF_HOME") or HOME / ".cache/huggingface") / "hub"


def download_speech_model(model):
    """Download one of Handy's single-file models into the Hugging Face cache
    layout Handy reads (blobs/, snapshots/<revision>/, refs/)."""
    import hashlib
    hf = model.get("source", {}).get("HuggingFace") or {}
    if model.get("is_directory") or not hf.get("repo_id"):
        sys.exit(f"{model['id']} can't be downloaded here; download it in Handy's settings, then re-run")
    repo, rev, name = hf["repo_id"], hf["revision"], model["filename"]
    root = hf_cache() / ("models--" + repo.replace("/", "--"))
    for d in ("blobs", f"snapshots/{rev}", "refs"):
        (root / d).mkdir(parents=True, exist_ok=True)
    part = root / "blobs" / f"{name}.part"
    print(f"   downloading {model['name']} ({model.get('size_mb', '?')} MB)", flush=True)
    sha = hashlib.sha256()
    with urllib.request.urlopen(f"https://huggingface.co/{repo}/resolve/{rev}/{name}") as resp, open(part, "wb") as f:
        while chunk := resp.read(1 << 20):
            sha.update(chunk)
            f.write(chunk)
    blob = root / "blobs" / sha.hexdigest()
    part.replace(blob)
    link = root / "snapshots" / rev / name
    if link.exists() or link.is_symlink():
        link.unlink()
    try:
        link.symlink_to(Path("../../blobs") / blob.name)
    except OSError:  # Windows without symlink rights
        shutil.copyfile(blob, link)
    (root / "refs" / rev).write_text(rev)


def read_env():
    try:
        lines = (KEY_FILE.parent / "env").read_text().splitlines()
    except OSError:
        return {}
    return dict(l.split("=", 1) for l in lines if "=" in l)


def prepare_handy(requested):
    """dictate uses Handy's engine, not its app: make sure a speech model is
    downloaded, and keep the app from running (its hotkeys would clash).
    Returns the model id to pass to Handy, or None for Handy's selected one."""
    step("Handy speech model")
    catalog = handy_models()
    if not catalog:
        sys.exit("couldn't list Handy's models (`handy --list-models --json`); is Handy installed?")
    downloaded = [m["id"] for m in catalog if m.get("is_downloaded")]
    chosen = None
    previous = read_env().get("LOCALFLOW_SPEECH_MODEL")
    if not requested and previous in downloaded:
        requested = previous  # keep what an earlier run set up
    if requested:
        chosen = next((m for m in catalog if m["id"] == requested), None)
        if not chosen:
            sys.exit(f"Handy has no model {requested!r}; see `handy --list-models`")
    elif not downloaded:
        chosen = next((m for m in catalog if m.get("is_recommended") and not m.get("is_directory")), None)
        chosen = chosen or next(m for m in catalog if not m.get("is_directory"))
    if chosen and chosen["id"] not in downloaded:
        download_speech_model(chosen)
        if not any(m["id"] == chosen["id"] and m.get("is_downloaded") for m in handy_models()):
            sys.exit(f"Handy doesn't see the downloaded {chosen['id']}; download it in Handy's settings")
    print(f"   speech model: {chosen['name'] if chosen else 'the one selected in Handy'}")
    stop_handy()
    store = handy_data_dir() / "settings_store.json"
    if store.exists():
        data = json.loads(store.read_text())
        data.setdefault("settings", {})["autostart_enabled"] = False
        store.write_text(json.dumps(data, indent=2))
    return chosen["id"] if chosen else None


def remove_proxy():
    """The proxy only serves Handy's app; dictate cleans up in-process."""
    if SYSTEM == "Linux":
        unit = HOME / ".config/systemd/user/localflow.service"
        if unit.exists():
            run("systemctl", "--user", "disable", "--now", "localflow.service", check=False)
    elif SYSTEM == "Darwin":
        plist = HOME / "Library/LaunchAgents/computer.localflow.plist"
        if plist.exists():
            subprocess.run(["launchctl", "bootout", f"gui/{os.getuid()}", str(plist)], capture_output=True)
            plist.unlink()
    else:
        cmd = Path(os.environ["APPDATA"]) / "Microsoft/Windows/Start Menu/Programs/Startup/localflow.cmd"
        if cmd.exists():
            cmd.unlink()
            print("   removed the proxy from Startup (it stops at your next sign-in)")


def has_tk(py):
    return subprocess.run([str(py), "-c", "import tkinter; tkinter.Tk().destroy()"],
                          capture_output=True, timeout=30).returncode == 0


def install_dictate(hyprland):
    step("dictate")
    py = Path(sys.executable)
    if SYSTEM != "Windows":
        DICTATE.chmod(0o755)
    if not has_tk(py):
        hint = {"Darwin": f"brew install python-tk@{sys.version_info.major}.{sys.version_info.minor}",
                "Windows": "re-run the Python installer with 'tcl/tk and IDLE' ticked"}.get(
            SYSTEM, "sudo pacman -S tk  /  sudo apt install python3-tk  /  sudo dnf install python3-tkinter")
        print(f"   no Tk for {py}: dictate will use notifications instead of the overlay ({hint})")
    if SYSTEM == "Darwin":
        if not have("rec") and not have("ffmpeg"):
            require("brew", "Install sox (https://sox.sourceforge.net) for recording")
            run("brew", "install", "sox")
        plist = HOME / "Library/LaunchAgents/computer.localflow.dictate.plist"
        plist.parent.mkdir(parents=True, exist_ok=True)
        path = os.pathsep.join(["/opt/homebrew/bin", "/usr/local/bin", "/usr/bin", "/bin"])
        plist.write_text(f"""<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
  <key>Label</key><string>computer.localflow.dictate</string>
  <key>ProgramArguments</key><array><string>{py}</string><string>{DICTATE}</string></array>
  <key>EnvironmentVariables</key><dict><key>PATH</key><string>{path}</string></dict>
  <key>RunAtLoad</key><true/>
  <key>KeepAlive</key><true/>
  <key>ProcessType</key><string>Interactive</string>
</dict></plist>
""")
        domain = f"gui/{os.getuid()}"
        subprocess.run(["launchctl", "bootout", domain, str(plist)], capture_output=True)
        time.sleep(1)
        run("launchctl", "bootstrap", domain, plist)
        print(f"   allow {py} in System Settings > Privacy & Security > Accessibility (hotkey and\n"
              "   paste) and Microphone (asked on first use), then: launchctl kickstart -k "
              f"{domain}/computer.localflow.dictate")
    elif SYSTEM == "Windows":
        pyw = py.with_name("pythonw.exe")
        pyw = pyw if pyw.exists() else py
        startup = Path(os.environ["APPDATA"]) / "Microsoft/Windows/Start Menu/Programs/Startup"
        (startup / "localflow-dictate.cmd").write_text(f'@echo off\r\nstart "" "{pyw}" "{DICTATE}"\r\n')
        subprocess.Popen([str(pyw), str(DICTATE)], creationflags=0x00000008)  # already running: it exits
    elif hyprland:
        print(f"""   Bind the keys in ~/.config/hypr/bindings.lua:
     hl.unbind("SUPER + CTRL + X")
     hl.unbind("F9")
     o.bind("SUPER + CTRL + X", "Toggle dictation", "{DICTATE} toggle")
     o.bind("F9", "Start dictation (push-to-talk)", "{DICTATE} start")
     o.bind("F9", "Stop dictation (push-to-talk)", "{DICTATE} stop", {{ release = true }})""")
    else:
        print(f"   Add keyboard shortcuts in your desktop's settings: `{DICTATE} toggle`\n"
              f"   (or `{DICTATE} start` on press and `{DICTATE} stop` on release).")


def require(cmd, hint):
    if not have(cmd):
        sys.exit(f"`{cmd}` not found. {hint}, then re-run install.py")


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--model", help="model for cleanup (default: qwen3.5:4b, or "
                                    "qwen3.5:4b-mlx on Apple Silicon; --api anthropic always uses "
                                    f"{HAIKU}; required with other APIs)")
    ap.add_argument("--api", metavar="PROVIDER_OR_URL",
                    help="use your own API key instead of Ollama: anthropic (Claude), openai, openrouter, "
                         "groq, gemini, mistral, deepseek, or any OpenAI-compatible base URL")
    ap.add_argument("--speech-model", metavar="ID",
                    help="Handy speech model to download and use (see `handy --list-models`; default: the "
                         "one selected in Handy, or Handy's recommended model if none is downloaded)")
    ap.add_argument("--handy-app", action="store_true",
                    help="use Handy's own app and hotkeys, with localflow as its post-processing proxy, "
                         "instead of dictate")
    ap.add_argument("--rocm", action=argparse.BooleanOptionalAction, default=None,
                    help="Linux: also install Ollama's ROCm build (default: auto-detect AMD GPU)")
    args = ap.parse_args()
    if args.api and args.api.lower() == "anthropic":
        if args.model not in (None, HAIKU):
            ap.error(f"--api anthropic always uses {HAIKU}; drop --model")
        args.model = HAIKU
    if args.api and not args.model:
        ap.error("--api needs --model (the provider's model name)")
    rocm = has_amd_gpu() if args.rocm is None else args.rocm
    hyprland = SYSTEM == "Linux" and have("hyprctl") and bool(os.environ.get("HYPRLAND_INSTANCE_SIGNATURE"))

    if args.api:
        model = args.model
        setup_api(args.api, model)
    else:
        install_ollama(rocm)
        model = pull_model(args.model)
    install_handy()
    if args.handy_app:
        install_proxy(model, hyprland, args.api)
        configure_handy(hyprland)
    else:
        speech = prepare_handy(args.speech_model)
        # dictate warms the model as you start speaking, so it can unload soon after.
        env = cleanup_env(model, args.api, keep_alive="2m")
        if speech:
            env["LOCALFLOW_SPEECH_MODEL"] = speech
        write_env(env)
        remove_proxy()
        install_dictate(hyprland)

    step(f"Done (cleanup model: {model}{f' via {args.api}' if args.api else ''})")
    if args.api and SYSTEM == "Linux" and subprocess.run(["systemctl", "--user", "is-active", "--quiet",
                                                          "ollama"]).returncode == 0:
        print("   Ollama isn't needed now: `systemctl --user disable --now ollama` frees its memory.")
    if not args.handy_app:
        keys = {"Darwin": "Option+Shift+Space", "Windows": "Ctrl+Shift+Space"}.get(SYSTEM)
        if keys:
            print(f"   Dictate: hold {keys} and talk (or tap it, talk, tap again). Esc cancels.")
        print(f"   Check everything: {sys.executable} {DICTATE} --selftest")
    elif SYSTEM == "Darwin":
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
