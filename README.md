# localflow

_(formerly handy-clean)_

Local, private dictation cleanup for [Handy](https://handy.computer), backed by
a small LLM in [Ollama](https://ollama.com). Works on macOS, Windows and Linux.

Handy turns speech into text; localflow turns that text into what you meant:

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
   a retraction left in ("pretty good, actually this is not pretty good, this
   is awesome"),
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
Handy ──(OpenAI API)──▶ localflow serve :11435 ──(native API)──▶ Ollama :11434 (qwen3.5:4b)
```

Handy's **Custom** post-processing provider points at the localflow proxy.
The proxy exists because Ollama's OpenAI-compatible endpoint ignores Modelfile
temperature (the same dictation came back different each time) and few-shot
history. The proxy calls Ollama's native API with temperature 0, the rules and
examples in `localflow`, your vocabulary from `words.txt`, and falls back to
the raw transcript if the model replies with something much longer than you
said or Ollama is down. You never lose a dictation.

## Requirements & resource use

Check these before installing. Numbers measured on Linux (Ryzen 5 5600, 6
cores; Radeon RX 9070 XT, 16 GB VRAM) with the default `qwen3.5:4b`.

**Disk** (about 4.5–7 GB in total)

| What | Size |
| --- | --- |
| Cleanup model `qwen3.5:4b` | 3.3 GB (`qwen3.5:2b`: 2.7 GB) |
| Ollama | ~2.4 GB on Linux, +2.2 GB for the ROCm (AMD GPU) build |
| Handy | ~0.5 GB |
| Handy speech model (you pick it in Handy) | ~0.5–0.7 GB for the Parakeet models |

**Memory while it's running**

| What | With a GPU | CPU only |
| --- | --- | --- |
| Cleanup model | ~3.1 GB of VRAM + ~1.1 GB RAM | ~3.6 GB RAM |
| Handy (speech model loaded only while dictating) | ~0.3 GB RAM | ~0.3 GB RAM |
| Ollama server + localflow proxy | ~0.1 GB RAM | ~0.1 GB RAM |

In total that's about 1.5 GB of RAM (plus VRAM) while the model is loaded,
and about 0.4 GB once it unloads. The model stays loaded for 1 hour after
your last dictation; the first dictation after that waits ~1.5-3 s while it
loads again (`LOCALFLOW_KEEP_ALIVE` changes the hour, e.g. `5m` to free the
memory sooner). Cloud dictation apps use less RAM because their models run on
their servers; here everything runs on your machine.

**Speed** (time from releasing the key to text appearing, after transcription)

| | Short sentence | Long ramble (~1 min of speech) |
| --- | --- | --- |
| GPU (RX 9070 XT) | ~0.4 s | ~1.5 s |
| CPU only (Ryzen 5 5600) | ~0.9 s | ~20 s |

**Recommended:** 8 GB RAM minimum, 16 GB comfortable; a GPU with 4 GB+ of
VRAM (NVIDIA, AMD or Apple Silicon) for fast long dictation. Without a GPU,
`--model qwen3.5:2b` roughly halves the wait.

**Software:** Python 3.9+ and git. On Linux also `curl` and `zstd` (to unpack
Ollama), an x86-64 CPU, and Handy's own needs (`webkit2gtk-4.1`,
`gtk-layer-shell`); on Hyprland the paste helper needs `wl-clipboard` and
`wtype` (Omarchy ships all of these).

## Install

```sh
git clone https://github.com/radiumcoders/localflow
cd localflow
python3 install.py                      # Windows: py install.py
```

`install.py` is safe to re-run. It:

1. installs Ollama if missing: Homebrew on macOS, winget on Windows, a per-user
   install in `~/.local/ollama` on Linux (no sudo; adds the ROCm build when an
   AMD GPU is found, `--no-rocm` to skip)
2. pulls the cleanup model, **qwen3.5:4b** (~3.3 GB). Optional: `--model qwen3.5:2b`
   for a faster, less accurate one
3. runs the proxy at login: launchd on macOS, the Startup folder on Windows,
   a systemd user service on Linux
4. installs Handy if missing: Homebrew cask, winget, or the AppImage on Linux
5. points Handy's post-processing at the proxy, and sets Handy to load its
   speech model only while you dictate (it loads in under a second, in the
   background as recording starts, and frees the memory right after)

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
There Handy keeps typing its output, so a dictated list's line breaks become
Enter presses; switch Handy's paste method to a clipboard paste if that bites.
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
echo "call mom tomorrow actually tonight" | ./localflow
./localflow --eval                 # run cases.txt against the current model
./localflow --eval qwen3.5:2b      # compare another Ollama model
./localflow --eval qwen3.5:4b off  # draft only (no repair call)
```

Rules and examples live in `SYSTEM` / `EXAMPLES` in `localflow`. After
editing, run `--eval`, then restart the proxy (`systemctl --user restart
localflow` on Linux, `launchctl kickstart -k gui/$(id -u)/computer.localflow`
on macOS, log out and in on Windows).

`cases.txt` holds `raw => expected` cases. Expected text is compared ignoring
case and punctuation; list cases compare the numbered items; `? +must; -must
not; order: a < b < c` checks properties of long, open-ended dictation.

Current results on an RX 9070 XT: `qwen3.5:4b` 32–33/36 (30–31 from the draft, +2 from review; review runs on ~1 in 15 dictations, +72 ms on average), ~0.3 s for short
dictation, ~1.5 s for long rambling ones; `qwen3.5:2b` is about twice as fast
and misses more corrections; `qwen3.5:9b` scored lower (26/31) at ~1.5–2x the latency.

## Known limitations

- Moving the project folder breaks the proxy service and the Hyprland paste
  helper (both point at it); re-run `install.py` from the new place.
- On Linux, Handy runs from an extracted AppImage, so Handy's in-app updates
  don't apply. To update: delete `~/.local/share/handy` and re-run `install.py`.
- The macOS and Windows install paths are written but not yet tested on real
  machines; Linux (Arch/Omarchy) is.
- Misheard words are only fixed reliably when they're in `words.txt` with a
  hint; from context alone a 4b model often can't tell (Javanese vs Japanese).

## Files

| File | What |
| --- | --- |
| `localflow` | rules + examples, `serve` proxy, stdin filter, `--eval` |
| `install.py` | cross-platform installer |
| `words.txt` | your names/terms, with optional mishearing hints |
| `cases.txt` | eval cases |
| `paste` | Linux/Hyprland paste helper for Handy |
