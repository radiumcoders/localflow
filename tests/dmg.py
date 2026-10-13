"""The macOS disk image, on a fresh Mac (CI): install the app from the .dmg,
set it up without its window, check it runs at login, then remove it.

    python tests/dmg.py build/localflow-<version>-<arch>.dmg

1. Opens the .dmg: localflow.app (validly signed, with its microphone
   message) and an Applications shortcut; copies the app to /Applications.
2. `localflow install` against a fake cleanup API (tests/fake_api.py) with a
   tiny speech model: setup's steps inside the packaged app, with no Homebrew
   needed.
3. The background app that setup registered with launchd: running, waiting
   for the hotkey (or for the Accessibility permission, as on a new Mac).
4. `localflow --selftest`: overlay, focus, clipboard, hotkey, microphone.
5. Removes the app and its login item.

Prints PASS / FAIL per step; exits 1 if anything failed.
"""

import os
import plistlib
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
from e2e import SPEECH_MODEL, background_app, failures, start_fake_api, step, stop_background  # noqa: E402

APP = Path("/Applications/localflow.app")
EXE = APP / "Contents/MacOS/localflow"
AGENT = Path.home() / "Library/LaunchAgents/computer.localflow.dictate.plist"


def install(dmg):
    mount = Path(tempfile.mkdtemp())
    subprocess.run(["hdiutil", "attach", "-nobrowse", "-quiet", "-mountpoint", str(mount), str(dmg)], check=True)
    try:
        assert (mount / "localflow.app").is_dir(), "no localflow.app in the .dmg"
        assert os.readlink(mount / "Applications") == "/Applications", "no Applications shortcut"
        shutil.rmtree(APP, ignore_errors=True)
        subprocess.run(["ditto", str(mount / "localflow.app"), str(APP)], check=True)
    finally:
        subprocess.run(["hdiutil", "detach", "-quiet", str(mount)])
    r = subprocess.run(["codesign", "--verify", "--deep", "--strict", str(APP)], capture_output=True, text=True)
    assert r.returncode == 0, f"signature: {r.stderr}"
    with open(APP / "Contents/Info.plist", "rb") as f:
        info = plistlib.load(f)
    assert info.get("NSMicrophoneUsageDescription"), "no microphone message (macOS would kill it on recording)"
    return f"{APP}, version {info.get('CFBundleShortVersionString')}, {info.get('CFBundleIdentifier')}"


def setup(api):
    env = dict(os.environ, LOCALFLOW_API_KEY="test-key")
    r = subprocess.run([str(EXE), "install", "--api", api, "--model", "fake-small", "--speech-model", SPEECH_MODEL],
                       env=env, timeout=1200)
    assert r.returncode == 0, f"localflow install exited {r.returncode}"
    with open(AGENT, "rb") as f:
        args = plistlib.load(f)["ProgramArguments"]
    assert args == [str(EXE), "--background"], f"login item runs {args}"
    return f"set up through the packaged app; login item runs {' '.join(args)}"


def selftest():
    stop_background()  # it holds the hotkey
    r = subprocess.run([str(EXE), "--selftest", "3"], timeout=180)
    assert r.returncode == 0, f"--selftest exited {r.returncode}"
    return "all checks passed"


def remove():
    stop_background()
    AGENT.unlink(missing_ok=True)
    shutil.rmtree(APP)
    return "app and login item removed"


def main():
    dmg = Path(sys.argv[1]).resolve()
    api = start_fake_api()
    step("install from the .dmg", lambda: install(dmg))
    if not failures:
        step("setup in the app", lambda: setup(api))
        step("background app at login", background_app)
        step("self-test of the installed app", selftest)
        step("remove", remove)
    print(f"\n{len(failures)} failed: {failures}" if failures else "\nall passed")
    sys.exit(1 if failures else 0)


if __name__ == "__main__":
    main()
