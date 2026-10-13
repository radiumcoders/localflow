"""Write parity.json: what the Python `localflow` returns for the functions the
Rust port has to match exactly (prompts, checks, headings, file references,
learning's diffs). tests/parity.rs reads it.

    python3 rust/crates/core/tests/parity.py
"""

import difflib
import importlib.machinery
import importlib.util
import json
import os
import random
import tempfile
from pathlib import Path

ROOT = Path(__file__).resolve().parents[4]
os.environ["XDG_CONFIG_HOME"] = os.environ["APPDATA"] = tempfile.mkdtemp()  # no learned words
for k in [k for k in os.environ if k.startswith("LOCALFLOW_")]:
    del os.environ[k]
loader = importlib.machinery.SourceFileLoader("localflow", str(ROOT / "localflow"))
lf = importlib.util.module_from_spec(importlib.util.spec_from_loader("localflow", loader))
loader.exec_module(lf)

rng = random.Random(7)
cases = []
for line in (ROOT / "cases.txt").read_text(encoding="utf-8").splitlines():
    if line.strip() and not line.startswith("#") and "=>" in line:
        raw, expected = (s.strip() for s in line.split("=>", 1))
        cases.append((raw, expected.replace("\\n", "\n")))
pairs = cases + [(raw, cleaned) for raw, cleaned, *_ in lf.EXAMPLES]

headings_in = [cleaned for _, cleaned in pairs] + [
    "On hiring:\nWe need two people.\n\nOn sales:\nWe're up 5%.",
    "On hiring:\nWe need two people.\n\nThat's all.",
    "Sales\nWe're up.\n\nHiring\nTwo more people.",
    "Sales\nWe're up.",
    "For the frontend side:\nNew settings page.\n\nregarding the backend:\nPostgres now.",
    "about Écoles:\nThey open.\n\nAbout the café front:\nIt closes.",
    "Intro line.\n\nThe Plan\nShip it.\n\nNext Steps And More Things\nTest.\n\nRisks\nNone.",
    "", "\n\n", "Title\n\nBody",
]

problem_pairs = pairs + [
    ("I will maybe come only on friday", "I will come on Friday."),
    ("never do that unless asked", "Never do that unless asked."),
    ("the food is good actually this is not good this is great",
     "The food is good, actually this is not good, this is great."),
    ("ask Claw about it", "Ask Claw about it."),
    ("ask claw about it", "Ask Claude about it."),
    ("open the py script editor", "Open the editor."),
    ("learn some Javanese", "Learn some Japanese."),
    ("wait no not that one, the other", "Wait, no, not that one, the other."),
]
hints = lf.misheard_hints()

plausible_cases = [
    ("Check @src/a/very/long/path/name/settings.py now.", "check settings now",
     ["src/a/very/long/path/name/settings.py"]),
    ("x" * 100, "short text", []),
    ("   ", "anything", []),
    ("Fine.", "fine", []),
    ("@a/b @a/b/c done", "a b c done", ["a/b", "a/b/c"]),
] + [(cleaned, raw, []) for raw, cleaned in pairs]

refs_cases = [
    ("Look at @install.py, @src/app.py. And @tests/? Mail me@example.com", ["install.py", "src/app.py", "tests/"]),
    ("@README.md", ["README.md"]),
    ("See @README.md.", ["README.md"]),
    ("See @README.md.", ["README.md."]),
    ("@unknown and @src/App.tsx)", ["src/App.tsx"]),
    ("no refs here", []),
    ("café @naïve.py!", ["naïve.py"]),
]


def mutate(sentence):
    words = sentence.split()
    for _ in range(rng.randint(0, 3)):
        if not words:
            break
        i, op = rng.randrange(len(words)), rng.random()
        if op < 0.5:
            words[i] = rng.choice(["Claude", "GitHub", "their", "Postgres", "the", "Vercel", "local flow"])
        elif op < 0.75:
            del words[i]
        else:
            words.insert(i, rng.choice(["really", "Slack", "and"]))
    return " ".join(words)


correction_cases = [
    ("Push it to git hub and ask Claw about local flow.", "Push it to GitHub and ask Claude about localflow."),
    ("I think there car is red", "I think their car is red"),
    ("Push it to git hub", "Something else entirely here"),
    ("Meet at: noon", "Meet at: Noon:"),
    ("", "x"),
] + [(cleaned, mutate(cleaned)) for _, cleaned in pairs for _ in range(3)]

region_cases = [
    ("Ask Claw to push it to git hub.", "Hi. Ask Claude to push it to GitHub. And more text after"),
    ("Ask Claw to push it to git hub.", "Something unrelated"),
    ("Ship it tuesday.", "Ship it wednesday. Thanks!"),
] + [(cleaned, f"{rng.choice(['Hi.', 'So', ''])} {mutate(cleaned)} {rng.choice(['Thanks!', 'and more', ''])}")
     for _, cleaned in pairs]

diff_cases = []
for _ in range(400):
    alphabet = rng.randint(2, 6)
    a = [rng.randrange(alphabet) for _ in range(rng.randint(0, 25))]
    b = [rng.randrange(alphabet) for _ in range(rng.randint(0, 25))]
    sm = difflib.SequenceMatcher(None, a, b, autojunk=False)
    diff_cases.append({"a": a, "b": b, "blocks": [list(m) for m in sm.get_matching_blocks()],
                       "opcodes": [list(o) for o in sm.get_opcodes()]})

grade_cases = [(expected, expected) for _, expected in cases] + [(raw, expected) for raw, expected in cases] + [
    ("1. A\n2. B", "Lead:\n1. a\n2. b"), ("Meet at five.", "Meet at 5."), ("One\nTwo", "One two"),
]

out = {
    "prefix": [{"role": m["role"], "content": m["content"]} for m in lf.prefix_messages(lf.rules())],
    "user_turn": [[t, f, lf.user_turn(t, f)] for t, f in [("hi", None), ("hi", ["a.py", "b/"]), ("x", [])]],
    "headings": [[s, lf.markdown_headings(s)] for s in headings_in],
    "problems": [[t, o, lf.find_problems(t, o)] for t, o in problem_pairs],
    "plausible": [[o, t, f, lf.plausible(o, t, f)] for o, t, f in plausible_cases],
    "file_refs": [[o, f, lf.file_refs(o, f)] for o, f in refs_cases],
    "corrections": [[a, b, lf.corrections(a, b)] for a, b in correction_cases],
    "edited_region": [[p, t, lf.edited_region(p, t)] for p, t in region_cases],
    "difflib": diff_cases,
    "grade": [[o, e, None] for o, e in grade_cases],  # filled below
}

# grade() lives inside run_eval; rebuild it the same way.
import re  # noqa: E402
digits = dict(zip("zero one two three four five six seven eight nine ten".split(), map(str, range(11))))
norm = lambda s: [digits.get(w, w) for w in re.sub(r"[^\w\s]", "", s.lower()).split()]  # noqa: E731
items = lambda t: [norm(l) for l in t.splitlines() if re.match(r"\s*\d+\.", l)]  # noqa: E731


def grade(o, e):
    if e.startswith("?"):
        return lf.check(o, e[1:])
    if items(e):
        return items(o) == items(e)
    return norm(o) == norm(e) and len(o.strip().splitlines()) == 1


out["grade"] = [[o, e, grade(o, e)] for o, e in grade_cases]
path = Path(__file__).with_name("parity.json")
path.write_bytes(json.dumps(out, ensure_ascii=False, indent=0).encode())  # UTF-8 and \n on Windows too
print(f"{path}: {sum(len(v) for v in out.values())} cases, hints {len(hints)}")
