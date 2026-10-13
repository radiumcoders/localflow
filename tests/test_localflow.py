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
        d.transcribe = lambda audio: "um call mom tomorrow"
        d.cleanup = lambda text: "Call mom tomorrow."
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

    def test_handy_override(self):
        os.environ["DICTATE_HANDY"] = "/opt/handy"
        try:
            self.assertEqual(self.d.handy_path(), "/opt/handy")
        finally:
            del os.environ["DICTATE_HANDY"]


if __name__ == "__main__":
    unittest.main()
