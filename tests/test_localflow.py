"""Tests that need no API key, model, microphone or display:
python -m unittest discover -s tests -v

A fake API server stands in for Anthropic and OpenAI-compatible providers."""

import importlib.machinery
import importlib.util
import array
import json
import math
import os
import sys
import tempfile
import threading
import unittest
import urllib.request
from http.server import ThreadingHTTPServer
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
sys.path.insert(0, str(ROOT / "tests"))
from fake_api import FakeAPI  # noqa: E402

CONFIG = tempfile.mkdtemp()  # no ~/.config/localflow/env from the machine running the tests


def load(name, env=None):
    """Import a script (no .py extension) with a clean LOCALFLOW_* environment."""
    for k in [k for k in os.environ if k.startswith("LOCALFLOW_")]:
        del os.environ[k]
    os.environ.update({"XDG_CONFIG_HOME": CONFIG, "APPDATA": CONFIG, **(env or {})})
    loader = importlib.machinery.SourceFileLoader(name, str(ROOT / name))
    spec = importlib.util.spec_from_loader(name, loader)
    module = importlib.util.module_from_spec(spec)
    loader.exec_module(module)
    return module


class LocalflowTest(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.server = ThreadingHTTPServer(("127.0.0.1", 0), FakeAPI)
        cls.url = f"http://127.0.0.1:{cls.server.server_address[1]}/v1"
        threading.Thread(target=cls.server.serve_forever, daemon=True).start()

    @classmethod
    def tearDownClass(cls):
        cls.server.shutdown()
        cls.server.server_close()

    def setUp(self):
        FakeAPI.seen.clear()

    def anthropic(self, **env):
        lf = load("localflow", {"LOCALFLOW_API_URL": "anthropic", "LOCALFLOW_API_KEY": "test-key", **env})
        lf.API_URL = self.url  # the fake server, still speaking the Anthropic API
        return lf

    def test_local_default(self):
        lf = load("localflow")
        self.assertEqual(lf.MODEL, "qwen3.5:4b")
        self.assertFalse(lf.API_URL)

    def test_anthropic_is_always_haiku(self):
        lf = self.anthropic(LOCALFLOW_MODEL="claude-opus-5-5", LOCALFLOW_REVIEW_MODEL="claude-sonnet-5-5")
        self.assertTrue(lf.ANTHROPIC)
        self.assertEqual((lf.MODEL, lf.REVIEW_MODEL), ("claude-haiku-5-5", "claude-haiku-5-5"))
        self.assertEqual(lf.clean("meet at 3 wait at 2"), "Meet at 3 wait at 2.")
        lf.chat("claude-opus-5-5", [{"role": "user", "content": "hi"}], 8)
        self.assertEqual({b["model"] for _, _, b in FakeAPI.seen}, {"claude-haiku-5-5"})

    def test_anthropic_request_shape(self):
        self.anthropic().clean("call mom")
        path, headers, body = FakeAPI.seen[0]
        self.assertEqual(path, "/v1/messages")
        self.assertEqual(headers.get("anthropic-version"), "2023-06-01")
        self.assertEqual(body["thinking"], {"type": "disabled"})
        self.assertNotIn("temperature", body)
        self.assertIn("dictation cleanup filter", body["system"])
        self.assertTrue(all(m["role"] in ("user", "assistant") for m in body["messages"]))
        # Rules and examples are cached: the marker sits on the last example.
        self.assertEqual(body["messages"][-2]["content"][0]["cache_control"], {"type": "ephemeral"})

    def test_bad_key_keeps_the_transcript(self):
        lf = self.anthropic(LOCALFLOW_API_KEY="wrong")
        with self.assertRaises(RuntimeError):
            lf.clean("keep my words")

    def test_openai_compatible(self):
        lf = load("localflow", {"LOCALFLOW_API_URL": self.url, "LOCALFLOW_MODEL": "small-model",
                                "LOCALFLOW_API_KEY": "k"})
        self.assertEqual(lf.clean("call mom tomorrow"), "Call mom tomorrow.")
        self.assertEqual(FakeAPI.seen[0][0], "/v1/chat/completions")
        self.assertEqual(FakeAPI.seen[0][2]["model"], "small-model")

    def test_provider_without_model_says_so(self):
        lf = load("localflow", {"LOCALFLOW_API_URL": "groq"})
        with self.assertRaisesRegex(RuntimeError, "LOCALFLOW_MODEL"):
            lf.chat(lf.MODEL, [{"role": "user", "content": "hi"}], 8)

    def test_env_file(self):
        Path(CONFIG, "localflow").mkdir(exist_ok=True)
        Path(CONFIG, "localflow", "env").write_text("LOCALFLOW_API_URL=anthropic\nNOT_OURS=1\n")
        try:
            lf = load("localflow")
            self.assertTrue(lf.ANTHROPIC)
            self.assertNotIn("NOT_OURS", os.environ)
            lf = load("localflow", {"LOCALFLOW_API_URL": "groq"})  # the environment wins
            self.assertFalse(lf.ANTHROPIC)
        finally:
            Path(CONFIG, "localflow", "env").unlink()

    def test_file_list_goes_with_the_dictation(self):
        lf = self.anthropic()
        lf.clean("look at the settings file", files=["src/settings.py", "tests/"])
        turn = FakeAPI.seen[0][2]["messages"][-1]["content"]
        self.assertIn("<dictation>look at the settings file</dictation>\n<files>\nsrc/settings.py\ntests/\n</files>",
                      turn if isinstance(turn, str) else turn[0]["text"])
        lf.clean("call mom")  # no files: no list
        self.assertNotIn("<files>", FakeAPI.seen[1][2]["messages"][-1]["content"])

    def test_file_refs(self):
        lf = load("localflow")
        files = ["install.py", "src/app.py", "tests/"]
        self.assertEqual(lf.file_refs("Look at @install.py, @src/app.py. And @tests/? Mail me@example.com", files),
                         [("text", "Look at "), ("file", "install.py"), ("text", ", "), ("file", "src/app.py"),
                          ("text", ". And "), ("file", "tests/"), ("text", "? Mail me@example.com")])
        self.assertTrue(lf.plausible("Check @src/a/very/long/path/name/settings.py now.", "check settings now",
                                     ["src/a/very/long/path/name/settings.py"]))

    def test_learn_corrections(self):
        lf = load("localflow")
        self.assertEqual(lf.corrections("Push it to git hub and ask Claw about local flow.",
                                        "Push it to GitHub and ask Claude about localflow."),
                         [("git hub", "GitHub"), ("Claw", "Claude"), ("local flow", "localflow")])
        self.assertEqual(lf.corrections("I think there car is red", "I think their car is red"), [])  # grammar
        self.assertIsNone(lf.corrections("Push it to git hub", "Something else entirely here"))
        words = Path(CONFIG, "localflow", "words.txt")
        try:
            self.assertEqual(lf.learn([("git hub", "GitHub"), ("slack", "Slack")]),
                             [("git hub", "GitHub"), ("slack", "Slack")])
            self.assertEqual(lf.learn([("git hub", "GitHub")]), [])  # already known
            self.assertEqual(lf.learn([("get hub", "GitHub")]), [("get hub", "GitHub")])
            self.assertIn('GitHub: often heard as "git hub", "get hub"', words.read_text())
            self.assertIn(("GitHub", ["git hub", "get hub"]), lf.misheard_hints())
            self.assertIn("- Slack", lf.rules())
        finally:
            words.unlink(missing_ok=True)

    def test_learns_from_what_you_send(self):
        """After a paste, the edited version you send to Claude Code is found in
        its transcript; the fixes the model calls mishearings are learned."""
        d = load("dictate", {"LOCALFLOW_API_URL": self.url, "LOCALFLOW_MODEL": "small", "LOCALFLOW_API_KEY": "k"})
        home = Path(tempfile.mkdtemp())
        d.HOME, d.RUN, d.LAST = home, home, home / "last.json"
        words = Path(CONFIG, "localflow", "words.txt")
        notes = []
        try:
            pasted = "Ask Claw to push it to git hub and ship it tuesday."
            stamp = d.remember(pasted)
            watcher = threading.Thread(target=d.watch, args=(pasted, None, stamp, notes.append))
            watcher.start()
            threading.Event().wait(0.5)
            log = home / ".claude/projects/-tmp-proj/session.jsonl"
            log.parent.mkdir(parents=True)
            sent = "Ask Claude to push it to GitHub and ship it wednesday. Thanks!"
            with open(log, "w") as f:
                f.write(json.dumps({"type": "user", "message": {"role": "assistant", "content": "x"}}) + "\n")
                f.write(json.dumps({"type": "user", "message": {"role": "user", "content": sent}}) + "\n")
            watcher.join(10)
            self.assertFalse(watcher.is_alive())
            # Claw -> Claude is already in words.txt; the fake model skips "wednesday" (lowercase).
            self.assertEqual(notes, ["Learned: git hub → GitHub"])
            self.assertIn(("GitHub", ["git hub"]), d.localflow().misheard_hints())
            judged = [b for _, _, b in FakeAPI.seen if "CHANGES:" in b["messages"][-1]["content"]]
            self.assertEqual(len(judged), 1)
        finally:
            words.unlink(missing_ok=True)

    def test_sent_messages_from_cursor_chat(self):
        import sqlite3
        d = load("dictate")
        d.HOME = Path(tempfile.mkdtemp())
        db = Path(CONFIG, "Cursor/User/globalStorage/state.vscdb") if sys.platform != "darwin" else \
            d.HOME / "Library/Application Support/Cursor/User/globalStorage/state.vscdb"
        db.parent.mkdir(parents=True, exist_ok=True)
        con = sqlite3.connect(db)
        con.execute("CREATE TABLE IF NOT EXISTS cursorDiskKV (key TEXT UNIQUE ON CONFLICT REPLACE, value BLOB)")
        con.execute("INSERT INTO cursorDiskKV VALUES ('bubbleId:a:1', ?)", (json.dumps({"type": 1, "text": "old"}),))
        con.commit()
        try:
            sent = d.SentMessages()
            self.assertEqual(sent.new(), [])
            con.execute("INSERT INTO cursorDiskKV VALUES ('bubbleId:a:2', ?)", (json.dumps({"type": 1, "text": "hi"}),))
            con.execute("INSERT INTO cursorDiskKV VALUES ('bubbleId:a:3', ?)", (json.dumps({"type": 2, "text": "ai"}),))
            con.commit()
            self.assertEqual(sent.new(), ["hi"])
            self.assertEqual(sent.new(), [])
        finally:
            con.close()
            db.unlink()

    def test_edited_region(self):
        lf = load("localflow")
        self.assertEqual(lf.edited_region("Ask Claw to push it to git hub.",
                                          "Hi. Ask Claude to push it to GitHub. And more text after"),
                         "Ask Claude to push it to GitHub.")
        self.assertIsNone(lf.edited_region("Ask Claw to push it to git hub.", "Something unrelated"))

    def test_proxy_for_handy(self):
        lf = self.anthropic()
        server = ThreadingHTTPServer(("127.0.0.1", 0), lf.Handler)
        threading.Thread(target=server.serve_forever, daemon=True).start()
        try:
            body = {"messages": [{"role": "user", "content": "<dictation>order three pizzas</dictation>"}]}
            req = urllib.request.Request(f"http://127.0.0.1:{server.server_address[1]}/v1/chat/completions",
                                         data=json.dumps(body).encode(), headers={"Content-Type": "application/json"})
            answer = json.load(urllib.request.urlopen(req, timeout=10))
            self.assertEqual(answer["choices"][0]["message"]["content"], "Order three pizzas.")
        finally:
            server.shutdown()
            server.server_close()


class DictateTest(unittest.TestCase):
    def setUp(self):
        self.d = load("dictate")

    def test_level(self):
        loud = array.array("h", (int(9000 * math.sin(i / 5)) for i in range(1600))).tobytes()
        self.assertEqual(self.d.level(bytes(3200)), 0.0)
        self.assertGreater(self.d.level(loud), 0.6)
        self.assertLessEqual(self.d.level(loud), 1.0)
        self.assertEqual(self.d.level(b""), 0.0)

    def test_dictation_flow(self):
        """Recording -> transcribe -> clean up -> paste, with stand-ins for the
        microphone, Handy, the cleanup and the paste."""
        import queue
        d, ui, pasted = self.d, queue.Queue(), []
        pcm = array.array("h", (int(9000 * math.sin(i / 5)) for i in range(16000))).tobytes()

        class Mic:
            def __init__(self, on_chunk):
                self.on_chunk = on_chunk

            def start(self):
                self.on_chunk(pcm[:3200])

            def stop(self):
                return pcm

        d.Recorder = Mic
        d.start_watching = lambda *args: None
        d.transcribe = lambda audio: "um call mom tomorrow"
        d.cleanup = lambda text, files=None: "Call mom tomorrow."
        d.warm_cleanup_model = lambda: None
        d.Paste.text = staticmethod(lambda text: pasted.append(text) or True)
        dic = d.Dictation(ui)
        dic.start()
        self.assertEqual(dic.finish(), 0)
        self.assertEqual(pasted, ["Call mom tomorrow."])
        ops = []
        while not ui.empty():
            ops.append(ui.get_nowait()[0])
        self.assertEqual([o for o in ops if o != "level"], ["recording", "working", "working", "hide"])

        raw = d.Dictation(ui, raw=True)  # raw: no cleanup
        raw.start()
        raw.finish()
        self.assertEqual(pasted[-1], "um call mom tomorrow")

        d.transcribe = lambda audio: ""
        silent = d.Dictation(ui)
        silent.start()
        self.assertGreater(silent.finish(), 0)  # shows "Nothing heard"
        self.assertEqual(len(pasted), 2)

    def test_too_short(self):
        import queue
        d = self.d

        class Blip:
            def __init__(self, on_chunk):
                pass

            def start(self):
                pass

            def stop(self):
                return bytes(1000)

        d.Recorder = Blip
        d.warm_cleanup_model = lambda: None
        dic = d.Dictation(queue.Queue())
        dic.start()
        self.assertGreater(dic.finish(), 0)

    def test_hotkey_hold_tap_escape(self):
        d = self.d
        log = []

        class Fake:
            def __init__(self):
                self.n = len(log)

            def start(self):
                log.append(("start", self.n))

            def cancel(self):
                log.append(("cancel", self.n))

        finished = threading.Event()

        def finish(dic):
            log.append(("finish", dic.n))
            finished.set()

        keys = d.HotkeyController(Fake, finish, lambda text: log.append(("message", text)))
        # Hold: press, release after 1 s -> finish.
        keys.on("press", 0.0)
        self.assertTrue(keys.recording())
        keys.on("release", 1.0)
        finished.wait(2)
        self.assertEqual(log[-1][0], "finish")
        self.assertFalse(keys.recording())
        for _ in range(100):  # the finish thread clears `working`
            if not keys.working:
                break
            threading.Event().wait(0.01)
        # Tap: press + quick release keeps recording; the next press finishes.
        finished.clear()
        keys.on("press", 10.0)
        keys.on("release", 10.1)
        self.assertTrue(keys.recording())
        keys.on("press", 15.0)
        finished.wait(2)
        keys.on("release", 15.1)  # release of the stopping tap: nothing happens
        self.assertEqual([e[0] for e in log], ["start", "finish", "start", "finish"])
        for _ in range(100):
            if not keys.working:
                break
            threading.Event().wait(0.01)
        # Escape cancels.
        keys.on("press", 20.0)
        keys.on("escape", 20.5)
        self.assertFalse(keys.recording())
        self.assertEqual(log[-1][0], "cancel")
        keys.on("release", 21.0)
        self.assertEqual(log[-1][0], "cancel")

    def test_hotkey_busy_and_errors(self):
        d = self.d
        messages = []

        class Broken:
            def start(self):
                raise RuntimeError("no microphone")

        keys = d.HotkeyController(Broken, lambda dic: None, messages.append)
        keys.on("press", 0.0)
        self.assertFalse(keys.recording())
        self.assertEqual(messages, ["no microphone"])
        keys.working = True  # still finishing the last one: presses are ignored
        keys.new_dictation = lambda: self.fail("started while busy")
        keys.on("press", 1.0)
        self.assertFalse(keys.recording())

    def test_file_candidates(self):
        files = ["README.md", "install.py", "src/settings.py", "src/components/Button.tsx",
                 "tests/test_settings.py", "src/", "src/components/", "tests/"]
        c = self.d.file_candidates
        self.assertEqual(c("refer to the install file", files)[0], "install.py")
        self.assertEqual(c("update the read me", files)[0], "README.md")
        self.assertEqual(c("check test settings dot py", files)[0], "tests/test_settings.py")
        self.assertIn("src/components/Button.tsx", c("look at the button component", files))
        self.assertEqual(c("we need to install the package", files), [])  # no word for a file
        self.assertEqual(c("look at the file", []), [])

    def test_deliver_by_app(self):
        d, pasted = self.d, []
        d.Paste.parts = staticmethod(lambda parts: pasted.append(parts) or True)
        d.Paste.text = staticmethod(lambda text: pasted.append(text) or True)
        root = Path(tempfile.mkdtemp())

        def target(kind, cwd=None):
            t = d.Target.__new__(d.Target)
            t.kind, t.app, t.project, t.cwd, t.files = kind, "x", root, cwd, []
            return t

        text, files = "Look at @src/app.py and @tests/ now.", ["src/app.py", "tests/"]
        d.deliver(text, target("terminal", root), files)
        self.assertEqual(pasted[-1], "Look at @src/app.py and @tests/ now.")
        d.deliver(text, target("terminal", root / "src"), files)
        self.assertEqual(pasted[-1], f"Look at @app.py and @{(root / 'tests').as_posix()}/ now.")
        d.deliver(text, target("editor"), files)
        self.assertEqual(pasted[-1], "Look at @src/app.py and @tests/ now.")  # text: nothing typed
        d.deliver(text, target("other"), files)
        self.assertEqual(pasted[-1], [("text", "Look at "), ("file", str(root / "src/app.py")),
                                      ("text", " and @tests/ now.")])  # a folder can't be attached
        d.deliver("No files here.", None, [])
        self.assertEqual(pasted[-1], "No files here.")

    def test_editor_project(self):
        d = self.d
        base = Path(tempfile.mkdtemp())
        a, b = base / "alpha", base / "beta"
        a.mkdir(), b.mkdir()
        if sys.platform == "darwin":
            d.HOME, state = base, base / "Library/Application Support/Cursor/User/globalStorage"
        else:  # XDG_CONFIG_HOME and APPDATA point at CONFIG
            state = Path(CONFIG, "Cursor/User/globalStorage")
        state.mkdir(parents=True, exist_ok=True)
        (state / "storage.json").write_text(json.dumps({"windowsState": {
            "lastActiveWindow": {"folder": a.as_uri()}, "openedWindows": [{"folder": b.as_uri()}]}}))
        try:
            self.assertEqual(d.editor_folders("Cursor"), [a, b])
            self.assertEqual(d.editor_project("Cursor", "● main.py - beta - Cursor"), b)
            self.assertEqual(d.editor_project("Cursor", "Welcome - Cursor"), a)  # the last one used
            t = d.Target("Cursor", "app.py - beta - Cursor")
            self.assertEqual((t.kind, t.project), ("editor", b))
        finally:
            (state / "storage.json").unlink()

    def test_project_files(self):
        root = Path(tempfile.mkdtemp())
        for f in ("a.py", "src/b.py", "src/ui/c.tsx", "node_modules/x/y.js", ".hidden/z"):
            (root / f).parent.mkdir(parents=True, exist_ok=True)
            (root / f).write_text("")
        self.assertEqual(sorted(self.d.project_files(root)), ["a.py", "src/", "src/b.py", "src/ui/", "src/ui/c.tsx"])
        self.assertEqual(self.d.project_root(root / "src"), root / "src")  # no git: the folder itself

    def test_learn_key(self):
        d, shown = self.d, []
        keys = d.HotkeyController(lambda: self.fail("no dictation"), lambda dic: None, shown.append,
                                  learn=lambda: "Learned: Claw → Claude")
        keys.on("learn", 0.0)
        for _ in range(100):
            if shown:
                break
            threading.Event().wait(0.01)
        self.assertEqual(shown, ["Learned: Claw → Claude"])

    def test_handy_override(self):
        os.environ["DICTATE_HANDY"] = "/opt/handy"
        try:
            self.assertEqual(self.d.handy_path(), "/opt/handy")
        finally:
            del os.environ["DICTATE_HANDY"]


class InstallTest(unittest.TestCase):
    def test_install_py_on_its_own_fetches_the_rest(self):
        import functools
        import shutil
        from http.server import SimpleHTTPRequestHandler
        handler = functools.partial(SimpleHTTPRequestHandler, directory=str(ROOT))
        handler.log_message = lambda *a: None
        server = ThreadingHTTPServer(("127.0.0.1", 0), handler)
        threading.Thread(target=server.serve_forever, daemon=True).start()
        self.addCleanup(server.shutdown)
        tmp = Path(tempfile.mkdtemp())
        shutil.copy(ROOT / "install.py", tmp / "install.py")
        loader = importlib.machinery.SourceFileLoader("lone_install", str(tmp / "install.py"))
        inst = importlib.util.module_from_spec(importlib.util.spec_from_loader("lone_install", loader))
        loader.exec_module(inst)
        inst.SOURCE = f"http://127.0.0.1:{server.server_address[1]}"
        inst.app_dir = lambda: tmp / "app"
        inst.fetch_app()
        self.assertEqual(inst.DICTATE, tmp / "app/dictate")
        for name in inst.APP_FILES:
            self.assertEqual((tmp / "app" / name).read_bytes(), (ROOT / name).read_bytes())
        if os.name != "nt":
            self.assertTrue(os.access(tmp / "app/paste", os.X_OK))
        self.assertEqual(list((tmp / "app").glob("*.part")), [])

    def test_install_py_in_a_clone_uses_the_clone(self):
        inst = load("install.py")
        inst.app_dir = lambda: self.fail("shouldn't download in a clone")
        inst.fetch_app()
        self.assertEqual(inst.DICTATE, ROOT / "dictate")


if __name__ == "__main__":
    unittest.main()
