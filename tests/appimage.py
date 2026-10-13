"""The Linux AppImage, on a fresh machine (CI): set it up without its window,
check the copy it runs from and the launcher entry, then self-test that copy.

    xvfb-run -a python tests/appimage.py build/localflow-<version>-x86_64.AppImage

1. `localflow.AppImage install` against a fake cleanup API (tests/fake_api.py)
   with a tiny speech model: setup's steps inside the packaged app.
2. The app copied to ~/.local/share/localflow/app (key bindings run it, so it
   must outlive the AppImage's mount), its launcher entry, and the settings.
3. The copy runs with the AppImage gone, and prints its key bindings.
4. `localflow --selftest` from the copy: overlay, clipboard, microphone, and
   HTTPS with the build machine's certificate paths hidden.

Prints PASS / FAIL per step; exits 1 if anything failed.
"""

import os
import subprocess
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
from e2e import SPEECH_MODEL, failures, start_fake_api, step  # noqa: E402

# The bundled OpenSSL's certificate paths are the build machine's, which CI
# machines have and users' machines may not: hide them, as on a user's machine.
NO_BUILD_CERTS = {"SSL_CERT_FILE": "/nonexistent/cert.pem", "SSL_CERT_DIR": "/nonexistent/certs"}

HOME = Path.home()
APP = Path(os.environ.get("XDG_DATA_HOME") or HOME / ".local/share") / "localflow/app"
DESKTOP = HOME / ".local/share/applications/localflow.desktop"
CONFIG = Path(os.environ.get("XDG_CONFIG_HOME") or HOME / ".config") / "localflow"


def setup(appimage, api):
    appimage.chmod(0o755)
    # CI machines have no FUSE; users' do (the AppImage mounts itself).
    env = dict(os.environ, LOCALFLOW_API_KEY="test-key", APPIMAGE_EXTRACT_AND_RUN="1")
    r = subprocess.run([str(appimage), "install", "--api", api, "--model", "fake-small",
                        "--speech-model", SPEECH_MODEL], env=env, timeout=1200)
    assert r.returncode == 0, f"install exited {r.returncode}"
    return "set up through the packaged app"


def installed():
    assert (APP / "localflow").exists(), f"no {APP}/localflow"
    entry = DESKTOP.read_text()
    assert f'Exec="{APP / "localflow"}"' in entry, entry
    settings = (CONFIG / "env").read_text()
    assert f"LOCALFLOW_SPEECH_MODEL={SPEECH_MODEL}" in settings, settings
    return f"{APP}, launcher entry, settings: {' '.join(settings.split())}"


def outlives(appimage):
    hidden = appimage.with_suffix(".away")
    appimage.rename(hidden)
    try:
        r = subprocess.run([str(APP / "localflow"), "localflow", "--check"], capture_output=True, text=True,
                           timeout=60)
        assert r.returncode == 0, r.stdout + r.stderr
        return r.stdout.strip()
    finally:
        hidden.rename(appimage)


def selftest():
    r = subprocess.run([str(APP / "localflow"), "--selftest", "3"], env=dict(os.environ, **NO_BUILD_CERTS),
                       timeout=180)
    assert r.returncode == 0, f"--selftest exited {r.returncode}"
    return "all checks passed"


def main():
    appimage = Path(sys.argv[1]).resolve()
    api = start_fake_api()
    step("setup in the app", lambda: setup(appimage, api))
    if not failures:
        step("installed app, launcher entry, settings", installed)
        step("runs without the AppImage", lambda: outlives(appimage))
        step("self-test of the installed app", selftest)
    print(f"\n{len(failures)} failed: {failures}" if failures else "\nall passed")
    sys.exit(1 if failures else 0)


if __name__ == "__main__":
    main()
