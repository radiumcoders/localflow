# handy-clean

Local, private dictation cleanup for [Handy](https://handy.computer), backed by
a small LLM in [Ollama](https://ollama.com). Works on macOS, Windows and Linux.

Handy turns speech into text; handy-clean turns that text into what you meant:

| You say | You get |
| --- | --- |
| "let's meet in 3 hours wait in 2 hours" | Let's meet in 2 hours. |
| "um I think we should uh ship it on Monday" | I think we should ship it on Monday. |
| "my tasks today are first buy milk second call the bank third fix the login bug" | My tasks today are:<br>1. Buy milk<br>2. Call the bank<br>3. Fix the login bug |
| "generate some code using Claw and push it to GitHub" | …using Claude… (from your `words.txt`) |

It handles self-corrections ("wait", "actually", "sorry I mean", "scratch that",
"I just remembered"), filler words, punctuation, numbered and nested lists in
long rambling speech, paragraphs and `## ` section headings for long
multi-topic dictation, spoken commands ("new paragraph", "new line",
"heading …", "bullet point …"), and misheard names. It never answers what you say: a
dictated question stays a question. Nothing leaves your machine.

## How it works

Two checkpoints:

1. **Draft**: the model cleans and formats the transcript, guided by the rules,
   worked examples and your `words.txt`.
2. **Review**: automatic checks look for concrete mistakes in the draft: a
   `words.txt` term still misheard ("Py script" left instead of Pine Script),
   or a qualifier you said ("maybe", "only", "never") that got dropped. If they
   find something, a fresh call (no examples, no history) repairs just that,
   and the repair is kept only if it clears the problem without adding new
   ones. Otherwise the draft goes out untouched, at no extra cost.

A free-form "review this draft" pass was tried first and measured worse: the
same 4b model fixed nothing and undid two correct self-corrections, and
qwen3.5:9b as reviewer did no better. Checks that never caught a real mistake
(leftover correction phrases, written-out spoken commands, similar-sounding
words) were removed.


```
Handy ──(OpenAI API)──▶ handy-clean serve :11435 ──(native API)──▶ Ollama :11434 (qwen3.5:4b)
```

Handy's **Custom** post-processing provider points at the handy-clean proxy.
The proxy exists because Ollama's OpenAI-compatible endpoint ignores Modelfile
temperature (the same dictation came back different each time) and few-shot
history. The proxy calls Ollama's native API with temperature 0, the rules and
examples in `handy-clean`, your vocabulary from `words.txt`, and falls back to
the raw transcript if the model replies with something much longer than you
said or Ollama is down. You never lose a dictation.

## Install

Needs Python 3.9+ (standard library only) and git.

```sh
git clone https://github.com/radiumcoders/handy-clean
cd handy-clean
python3 install.py                      # Windows: py install.py
```

`install.py` is safe to re-run. It:

1. installs Ollama if missing: Homebrew on macOS, winget on Windows, a per-user
   install in `~/.local/ollama` on Linux (no sudo; adds the ROCm build when an
   AMD GPU is found, `--no-rocm` to skip)
2. pulls the model (`--model qwen3.5:2b` for a faster, less accurate one)
3. runs the proxy at login: launchd on macOS, the Startup folder on Windows,
   a systemd user service on Linux
4. installs Handy if missing: Homebrew cask, winget, or the AppImage on Linux
5. points Handy's post-processing at the proxy

Pick a speech model in Handy's settings the first time you open it.

## Dictating

Handy only runs the cleanup on its **post-process** shortcut:

| Platform | Shortcut |
| --- | --- |
| macOS | `Option+Shift+Space` |
| Windows | `Ctrl+Shift+Space` |
| Linux | bind a key to `handy --toggle-post-process` (below) |

Handy's plain transcribe shortcut gives the raw text, handy for comparing.

### Linux (Hyprland / Omarchy)

Wayland apps can't grab global shortcuts, so let the compositor own the keys.
On Hyprland, `install.py` moves Handy's own shortcuts out of the way (they grab
Ctrl+Space and start stray recordings) and sets Handy to paste through
`./paste`: Ctrl+V in apps, Shift+Insert in terminals, like Omarchy's universal
paste, restoring your clipboard after. In `~/.config/hypr/bindings.lua`:

```lua
hl.unbind("SUPER + CTRL + X")
hl.unbind("F9")
o.bind("SUPER + CTRL + X", "Toggle dictation", "~/.local/bin/handy --toggle-post-process")
o.bind("F9", "Start dictation (push-to-talk)", "~/.local/bin/handy --toggle-post-process")
o.bind("F9", "Stop dictation (push-to-talk)", "~/.local/bin/handy --toggle-post-process", { release = true })
-- raw Handy output, no cleanup
o.bind("F10", "Start raw dictation (push-to-talk)", "~/.local/bin/handy --toggle-transcription")
o.bind("F10", "Stop raw dictation (push-to-talk)", "~/.local/bin/handy --toggle-transcription", { release = true })
```

and in `autostart.lua`: `o.launch_on_start("~/.local/bin/handy --start-hidden")`.

On GNOME/KDE, add a custom shortcut running `handy --toggle-post-process`.
If a recording ever gets stuck, `handy --cancel`.

## Your vocabulary

`words.txt` lists names and terms you say, one per line, optionally with a hint
about how they get misheard:

```
Claude: the AI assistant, often heard as "Claw" or "clawed"
Pine Script: TradingView's scripting language, often heard as "Py script"
```

It's read on every dictation; no restart needed.

## Tuning

```sh
echo "call mom tomorrow actually tonight" | ./handy-clean
./handy-clean --eval                 # run cases.txt against the current model
./handy-clean --eval qwen3.5:2b      # compare another Ollama model
./handy-clean --eval qwen3.5:4b off  # draft only (no repair call)
```

Rules and examples live in `SYSTEM` / `EXAMPLES` in `handy-clean`. After
editing, run `--eval`, then restart the proxy (`systemctl --user restart
handy-clean` on Linux, `launchctl kickstart -k gui/$(id -u)/computer.handy-clean`
on macOS, log out and in on Windows).

`cases.txt` holds `raw => expected` cases. Expected text is compared ignoring
case and punctuation; list cases compare the numbered items; `? +must; -must
not; order: a < b < c` checks properties of long, open-ended dictation.

Current results on an RX 9070 XT: `qwen3.5:4b` 30/33 (29 from the draft, +1 from review; review runs on ~1 in 15 dictations, +87 ms on average), ~0.3 s for short
dictation, ~1.5 s for long rambling ones; `qwen3.5:2b` is about twice as fast
and misses more corrections; `qwen3.5:9b` scored lower (26/31) at ~1.5–2x the latency.

## Files

| File | What |
| --- | --- |
| `handy-clean` | rules + examples, `serve` proxy, stdin filter, `--eval` |
| `install.py` | cross-platform installer |
| `words.txt` | your names/terms, with optional mishearing hints |
| `cases.txt` | eval cases |
| `paste` | Linux/Hyprland paste helper for Handy |
