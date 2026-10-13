"""localflow, the packaged app (built by pkg/build.py).

    localflow.exe                  # the first time: setup; then dictate in the background
                                   # (opened again while running: setup, to change settings)
    localflow.exe setup            # the setup window
    localflow.exe install ARGS     # setup without the window: install.py's options
    localflow.exe localflow ARGS   # the cleanup script: localflow --check, ...
    localflow.exe ARGS             # dictate's commands: --selftest, ...
"""

import socket
import sys


def log_to_file():
    """A windowed app has no console: keep what it prints in a log file."""
    import dictate
    dictate.STATE.mkdir(parents=True, exist_ok=True)
    sys.stdout = sys.stderr = open(dictate.STATE / "app.log", "a", encoding="utf-8", buffering=1)


def configured():
    import install
    return (install.KEY_FILE.parent / "env").exists()


def running():
    """dictate holds this port while it runs in the background."""
    s = socket.socket()
    try:
        s.bind(("127.0.0.1", 11437))
        return False
    except OSError:
        return True
    finally:
        s.close()


def main():
    if sys.stdout is None:
        log_to_file()
    args = sys.argv[1:]
    if args[:1] == ["localflow"]:
        import localflow
        localflow.main(args[1:])
    elif args[:1] == ["install"]:
        import install
        install.CAPTURE = True  # commands' output into the log, and no console windows
        install.main(args[1:])
    elif args[:1] == ["setup"] or not args and (not configured() or running()):
        import setup_app
        setup_app.main()
    else:
        import dictate
        dictate.main()


if __name__ == "__main__":
    main()
