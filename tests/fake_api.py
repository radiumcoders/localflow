"""A fake cleanup API for tests: Anthropic Messages and OpenAI-compatible chat.
It "cleans up" by capitalizing the dictation and adding a period.

    python tests/fake_api.py PORT      # serve until killed
"""

import json
import re
import sys
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer


class FakeAPI(BaseHTTPRequestHandler):
    """Anthropic Messages and OpenAI chat; replies with the dictation, capitalized."""
    seen = []

    def _send(self, code, obj):
        data = json.dumps(obj).encode()
        self.send_response(code)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(data)))
        self.end_headers()
        self.wfile.write(data)

    def do_POST(self):
        body = json.loads(self.rfile.read(int(self.headers["Content-Length"])))
        FakeAPI.seen.append((self.path, {k.lower(): v for k, v in self.headers.items()}, body))
        last = body["messages"][-1]["content"]
        last = "".join(b["text"] for b in last) if isinstance(last, list) else last
        m = re.search(r"<dictation>(.*)</dictation>", last, re.S)
        text = (m.group(1) if m else last).strip()
        reply = text[:1].upper() + text[1:].rstrip(".") + "."
        if "CHANGES:" in last:  # judging edits: learn the ones with a capital letter (names)
            changes = re.findall(r'^(\d+)\. ".*" -> "(.*)"$', last, re.M)
            reply = json.dumps({"learn": [int(n) for n, new in changes if new[:1].isupper()]})
        if self.path == "/v1/messages":
            if self.headers.get("x-api-key") != "test-key":
                return self._send(401, {"type": "error", "error": {"type": "authentication_error"}})
            return self._send(200, {"stop_reason": "end_turn", "content": [{"type": "text", "text": reply}]})
        return self._send(200, {"choices": [{"finish_reason": "stop", "message": {"content": reply}}]})

    def log_message(self, *args):
        pass


if __name__ == "__main__":
    ThreadingHTTPServer(("127.0.0.1", int(sys.argv[1])), FakeAPI).serve_forever()
