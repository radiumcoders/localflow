"""The Windows installer, on a fresh machine (CI): install the .msi, set up the
installed app without its window, check it runs at sign-in, then uninstall.

    python tests/msi.py build/localflow-<version>-x64.msi

1. Installs the .msi: the app in Program Files, a Start menu shortcut, and a
   Run entry so it starts at sign-in.
2. `localflow.exe install` against a fake cleanup API (tests/fake_api.py) with
   a tiny speech model: setup's steps inside the packaged app.
3. The background app that setup started: holds its lock port, hotkey ready.
4. `localflow.exe --selftest`: overlay, focus, clipboard, hotkey.
5. Uninstalls, and checks all of step 1 is gone.

Prints PASS / FAIL per step; exits 1 if anything failed.
"""

import os
import subprocess
import sys
import time
import winreg
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
from e2e import SPEECH_MODEL, background_app, failures, start_fake_api, step  # noqa: E402

APP = Path(os.environ["ProgramFiles"]) / "localflow/localflow.exe"
SHORTCUT = Path(os.environ["ProgramData"]) / "Microsoft/Windows/Start Menu/Programs/localflow.lnk"
LOGS = Path(os.environ["LOCALAPPDATA"]) / "localflow"
RUN_KEY = r"Software\Microsoft\Windows\CurrentVersion\Run"


def run_entry():
    try:
        with winreg.OpenKey(winreg.HKEY_CURRENT_USER, RUN_KEY) as k:
            return winreg.QueryValueEx(k, "localflow")[0]
    except OSError:
        return None


def app_log():
    path = LOGS / "app.log"
    return path.read_text(errors="replace") if path.exists() else "(no app.log)"


def msiexec(*args):
    log = Path(os.environ["RUNNER_TEMP"] if "RUNNER_TEMP" in os.environ else ".") / "msiexec.log"
    r = subprocess.run(["msiexec", *args, "/qn", "/l*v", str(log)], timeout=600)
    if r.returncode:
        tail = log.read_text(encoding="utf-16", errors="replace")[-3000:] if log.exists() else ""
        raise RuntimeError(f"msiexec {args[0]} exited {r.returncode}\n{tail}")


def install(msi):
    msiexec("/i", str(msi))
    assert APP.exists(), f"no {APP}"
    assert SHORTCUT.exists(), f"no Start menu shortcut at {SHORTCUT}"
    entry = run_entry()
    assert entry and str(APP).lower() in entry.lower(), f"Run entry: {entry!r}"
    return f"{APP}, Start menu shortcut, starts at sign-in ({entry})"


def setup(api):
    env = dict(os.environ, LOCALFLOW_API_KEY="test-key")
    r = subprocess.run([str(APP), "install", "--api", api, "--model", "fake-small", "--speech-model", SPEECH_MODEL],
                       env=env, timeout=1200)
    print(app_log()[-4000:])
    assert r.returncode == 0, f"localflow.exe install exited {r.returncode}"
    return "set up through the packaged app"


def selftest():
    subprocess.run(["taskkill", "/IM", "localflow.exe", "/F"], capture_output=True)  # it holds the hotkey
    time.sleep(1)
    start = len(app_log())
    r = subprocess.run([str(APP), "--selftest", "3"], timeout=120)
    print(app_log()[start:])
    assert r.returncode == 0, f"--selftest exited {r.returncode}"
    return "all checks passed"


def uninstall(msi):
    subprocess.run(["taskkill", "/IM", "localflow.exe", "/F"], capture_output=True)
    msiexec("/x", str(msi))
    for _ in range(10):  # Program Files can lag a moment behind msiexec
        if not APP.exists():
            break
        time.sleep(1)
    assert not APP.exists(), f"{APP} still there"
    assert not SHORTCUT.exists(), "Start menu shortcut still there"
    assert run_entry() is None, "Run entry still there"
    return "app, shortcut and Run entry removed"


def main():
    for stream in (sys.stdout, sys.stderr):
        stream.reconfigure(encoding="utf-8", errors="backslashreplace")
    msi = Path(sys.argv[1]).resolve()
    api = start_fake_api()
    step("install the .msi", lambda: install(msi))
    if not failures:
        step("setup in the app", lambda: setup(api))
        step("background app at sign-in", background_app)
        step("self-test of the installed app", selftest)
    step("uninstall", lambda: uninstall(msi))
    print(f"\n{len(failures)} failed: {failures}" if failures else "\nall passed")
    sys.exit(1 if failures else 0)


if __name__ == "__main__":
    main()
