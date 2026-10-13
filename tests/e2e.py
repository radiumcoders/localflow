"""End to end, on a fresh machine (CI): install, then dictate a spoken sentence
through the real pipeline and check it lands in a text box.

    python tests/e2e.py

1. Runs install.py against a fake cleanup API (tests/fake_api.py) with a tiny
   speech model, so it needs no key and downloads ~45 MB.
2. Speaks a sentence with the OS's text-to-speech into a WAV file.
3. On macOS / Windows: checks the background app that install.py started, then
   stops it.
4. Opens a text box in another process, and dictates into it: the hotkey
   (injected key presses, on macOS / Windows), the overlay, Handy's
   transcription, the cleanup and the paste. Only the microphone is replaced,
   by the WAV file.

Prints PASS / FAIL per step; exits 1 if anything failed.
"""

import importlib.machinery
import importlib.util
import json
import os
import platform
import queue
import shutil
import socket
import subprocess
import sys
import tempfile
import threading
import time
import wave
from http.server import ThreadingHTTPServer
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
sys.path.insert(0, str(ROOT / "tests"))
SYSTEM = platform.system()
SENTENCE = "Please call mom tomorrow at five."
SPEECH_MODEL = "handy-computer/whisper-tiny.en-gguf/whisper-tiny.en-Q8_0.gguf"
TMP = Path(tempfile.mkdtemp())
failures = []


def step(name, fn):
    print(f"\n--- {name}", flush=True)
    try:
        detail = fn()
        shown = f"{len(detail) / 32000:.1f}s of audio" if isinstance(detail, bytes) else detail
        print(f"PASS {name}{': ' + str(shown) if shown else ''}", flush=True)
        return detail
    except Exception as e:
        failures.append(name)
        print(f"FAIL {name}: {type(e).__name__}: {e}", flush=True)
        return None


def load(name):
    loader = importlib.machinery.SourceFileLoader(name, str(ROOT / name))
    spec = importlib.util.spec_from_loader(name, loader)
    module = importlib.util.module_from_spec(spec)
    loader.exec_module(module)
    return module


# --- 1. install ---------------------------------------------------------------------

def start_fake_api():
    from fake_api import FakeAPI
    server = ThreadingHTTPServer(("127.0.0.1", 0), FakeAPI)
    threading.Thread(target=server.serve_forever, daemon=True).start()
    return f"http://127.0.0.1:{server.server_address[1]}/v1"


def install(api):
    env = dict(os.environ, LOCALFLOW_API_KEY="test-key", PYTHONUNBUFFERED="1")
    r = subprocess.run([sys.executable, str(ROOT / "install.py"), "--api", api, "--model", "fake-small",
                        "--speech-model", SPEECH_MODEL], env=env, timeout=1200)
    if r.returncode:
        raise RuntimeError(f"install.py exited {r.returncode}")
    cfg = (Path(os.environ["APPDATA"]) if SYSTEM == "Windows" else
           Path(os.environ.get("XDG_CONFIG_HOME") or Path.home() / ".config")) / "localflow/env"
    text = cfg.read_text()
    assert f"LOCALFLOW_SPEECH_MODEL={SPEECH_MODEL}" in text, text
    # The key is read from the saved file from now on, as it would be after a reboot.
    os.environ.pop("LOCALFLOW_API_KEY", None)
    return f"config: {' '.join(text.split())}"


# --- 2. speech ----------------------------------------------------------------------

def speak():
    wav = TMP / "speech.wav"
    if SYSTEM == "Darwin":
        aiff = TMP / "speech.aiff"
        subprocess.run(["say", "-o", str(aiff), SENTENCE], check=True)
        subprocess.run(["afconvert", "-f", "WAVE", "-d", "LEI16@16000", "-c", "1", str(aiff), str(wav)], check=True)
    elif SYSTEM == "Windows":
        ps = f"""Add-Type -AssemblyName System.Speech
$s = New-Object System.Speech.Synthesis.SpeechSynthesizer
$f = New-Object System.Speech.AudioFormat.SpeechAudioFormatInfo(16000, [System.Speech.AudioFormat.AudioBitsPerSample]::Sixteen, [System.Speech.AudioFormat.AudioChannel]::Mono)
$s.SetOutputToWaveFile('{wav}', $f); $s.Speak('{SENTENCE}'); $s.Dispose()"""
        subprocess.run(["powershell", "-NoProfile", "-Command", ps], check=True)
    else:
        raw = TMP / "raw.wav"
        subprocess.run(["espeak-ng", "-w", str(raw), SENTENCE], check=True)
        subprocess.run(["ffmpeg", "-loglevel", "error", "-y", "-i", str(raw), "-ar", "16000", "-ac", "1",
                        "-sample_fmt", "s16", str(wav)], check=True)
    with wave.open(str(wav)) as w:
        assert (w.getframerate(), w.getnchannels(), w.getsampwidth()) == (16000, 1, 2), w.getparams()
        pcm = w.readframes(w.getnframes())
    return pcm


# --- 3. background app (macOS / Windows) ---------------------------------------------

def log_path():
    if SYSTEM == "Windows":
        return Path(os.environ["LOCALAPPDATA"]) / "localflow/dictate.log"
    return Path.home() / "Library/Logs/localflow/dictate.log"


def background_app():
    """install.py started dictate in the background; it holds a lock port and logs "ready"."""
    for _ in range(60):
        if "ready: hold" in (log_path().read_text(errors="replace") if log_path().exists() else ""):
            break
        time.sleep(0.5)
    text = log_path().read_text(errors="replace") if log_path().exists() else "(no log)"
    s = socket.socket()
    try:
        s.bind(("127.0.0.1", 11437))
        running = False
    except OSError:
        running = True
    finally:
        s.close()
    stop_background()
    assert running, f"not running; log:\n{text[-1500:]}"
    if "ready: hold" in text:
        return "running at login, hotkey ready (stopped it for the next test)"
    if SYSTEM == "Darwin" and "Accessibility" in text:
        # Expected until the user allows it (System Settings > Privacy & Security > Accessibility);
        # CI machines don't grant it to apps started at login.
        return "running at login; hotkey waits for the Accessibility permission (as on a new Mac)"
    raise AssertionError(f"running but the hotkey isn't ready; log:\n{text[-1500:]}")


def stop_background():
    if SYSTEM == "Windows":
        subprocess.run(["taskkill", "/IM", "pythonw.exe", "/F"], capture_output=True)
    else:
        subprocess.run(["launchctl", "bootout", f"gui/{os.getuid()}/computer.localflow.dictate"],
                       capture_output=True)
    time.sleep(1)


# --- 4. dictate into a text box ---------------------------------------------------------

def target_window(out, flag):
    """Child process: a focused text box. When `flag` appears, writes what it got
    to `out` and exits. Logs what happens, for when it doesn't work."""
    import tkinter
    t0 = time.monotonic()

    def say(msg):
        print(f"  [text box {time.monotonic() - t0:5.1f}s] {msg}", flush=True)

    root = tkinter.Tk()
    root.title("e2e target")
    root.geometry("500x200+100+100")
    text = tkinter.Text(root)
    text.pack(fill="both", expand=True)
    text.bind("<Key>", lambda e: say(f"key {e.keysym!r} state={e.state}"), add=True)

    def grab():
        root.lift()
        root.attributes("-topmost", True)
        if SYSTEM == "Windows":
            # Windows only lets the foreground app hand over the foreground; borrow its
            # input queue for a moment. (A lone Alt press would open the window menu.)
            u, k = ctypes.windll.user32, ctypes.windll.kernel32
            hwnd = int(root.wm_frame(), 16)
            fg_thread = u.GetWindowThreadProcessId(u.GetForegroundWindow(), None)
            me = k.GetCurrentThreadId()
            u.AttachThreadInput(me, fg_thread, True)
            u.SetForegroundWindow(hwnd)
            u.BringWindowToTop(hwnd)
            u.AttachThreadInput(me, fg_thread, False)
            say(f"foreground is ours: {u.GetForegroundWindow() == hwnd}")
        root.focus_force()
        text.focus_set()
        say(f"focus: {root.focus_get()}")

    def poll():
        if Path(flag).exists() or time.monotonic() - t0 > 170:
            got = text.get("1.0", "end").strip()
            say(f"done, got {got!r}")
            Path(out).write_text(got, encoding="utf-8")
            root.destroy()
        else:
            root.after(200, poll)

    root.after(300, grab)
    root.after(1500, grab)
    root.after(200, poll)
    root.mainloop()


import ctypes  # noqa: E402  (used by target_window on Windows)


class FileMic:
    """Stands in for the microphone: plays the WAV into the recorder in real time."""
    pcm = b""

    def __init__(self, on_chunk):
        self.on_chunk, self.stopped = on_chunk, False

    def start(self):
        def feed():
            for i in range(0, len(self.pcm), 3200):
                if self.stopped:
                    break
                self.on_chunk(self.pcm[i:i + 3200])
                time.sleep(0.1)
        threading.Thread(target=feed, daemon=True).start()

    def stop(self):
        self.stopped = True
        return self.pcm


def press_hotkey(down):
    """Inject the dictation hotkey (it's ignored unless DICTATE_TEST_INJECTED is set)."""
    if SYSTEM == "Windows":
        u = ctypes.windll.user32
        keys = [(0x11, 0), (0x10, 0), (0x20, 0)] if down else [(0x20, 2), (0x10, 2), (0x11, 2)]
        for vk, flag in keys:
            u.keybd_event(vk, 0, flag, 0)
    else:
        d = load("dictate")
        q = d.Quartz.lib()
        src = q.CGEventSourceCreate(1)
        e = q.CGEventCreateKeyboardEvent(src, 49, down)  # space
        q.CGEventSetFlags(e, d.Quartz.ALT | d.Quartz.SHIFT)
        q.CGEventPost(0, e)
        q.CFRelease(e)


def dictate_into_text_box(pcm):
    os.environ["DICTATE_TEST_INJECTED"] = "1"
    d = load("dictate")
    d.setup_logging(to_file=False)
    FileMic.pcm = pcm
    d.Recorder = FileMic
    out, flag = TMP / "typed.txt", TMP / "pasted"
    target = subprocess.Popen([sys.executable, __file__, "--target", str(out), str(flag)])
    time.sleep(2.5)  # let it take focus

    ui, events = queue.Queue(), queue.Queue()
    root, overlay = d.make_ui()
    assert root is not None, "no Tk overlay"
    shown, done = [], threading.Event()

    def finish(dic):
        try:
            dic.finish()
        finally:
            done.set()

    keys = d.HotkeyController(lambda: d.Dictation(ui), finish, lambda t: ui.put(("message", t, 3000)))
    hotkey = SYSTEM in ("Windows", "Darwin")
    if hotkey:
        listener = d.windows_hotkey if SYSTEM == "Windows" else d.mac_hotkey
        threading.Thread(target=lambda: listener(events, keys.recording), daemon=True).start()

    t0 = time.monotonic()
    plan = {"pressed": False, "released": False}

    def tick():
        now = time.monotonic() - t0
        if overlay.state != "hidden":
            shown.append(overlay.state)
        if hotkey:
            while not events.empty():
                ev = events.get_nowait()
                if ev != "ready":
                    keys.on(ev)
            if now > 0.5 and not plan["pressed"]:
                plan["pressed"] = True
                press_hotkey(True)
            if now > 0.5 + len(pcm) / 32000 + 0.5 and not plan["released"]:
                plan["released"] = True
                press_hotkey(False)
        else:  # Linux: what `dictate start` / `dictate stop` do, in-process
            if not plan["pressed"]:
                plan["pressed"] = True
                keys.on("press", 0.0)
            if now > len(pcm) / 32000 + 0.5 and not plan["released"]:
                plan["released"] = True
                keys.on("release", 10.0)
        return not done.is_set() and now < 150

    d.run_ui(root, overlay, ui, tick)
    time.sleep(1)  # let the paste arrive
    flag.touch()
    try:
        target.wait(20)
    except subprocess.TimeoutExpired:
        target.kill()
        raise AssertionError("the text box hung (see its log above)")
    typed = out.read_text(encoding="utf-8") if out.exists() else ""
    states = sorted(set(shown))
    assert plan["pressed"] and done.is_set(), f"dictation didn't finish (hotkey events: {plan}, overlay: {states})"
    assert "recording" in states and "working" in states, f"overlay states seen: {states}"
    assert "mom" in typed.lower(), f"text box got {typed!r}"
    return f"text box got {typed!r} (overlay showed {states})"


def main():
    if sys.argv[1:2] == ["--target"]:
        return target_window(sys.argv[2], sys.argv[3])
    print(f"e2e on {SYSTEM} {platform.release()}, Python {platform.python_version()}")
    api = start_fake_api()
    step("install.py", lambda: install(api))
    pcm = step("text-to-speech WAV", speak)
    if SYSTEM in ("Windows", "Darwin"):
        step("background app at login", background_app)
    if pcm:
        step("transcribe with Handy", lambda: f"{load('dictate').transcribe(pcm)!r}")
        step("dictate into a text box", lambda: dictate_into_text_box(pcm))
    print(f"\n{len(failures)} failed: {failures}" if failures else "\nall passed")
    sys.exit(1 if failures else 0)


if __name__ == "__main__":
    main()
