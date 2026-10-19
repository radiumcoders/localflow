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
dictated question stays a question. Nothing leaves your machine (unless you
opt into [your own API key](#use-your-own-api-key-optional)).

## Sponsors

<a href="https://www.shadcn-labs.com/?utm_source=localflow&utm_medium=referral&utm_campaign=backed_by">
  <picture>
    <source media="(prefers-color-scheme: dark)" srcset="assets/shadcn-labs-light.svg">
    <img src="assets/shadcn-labs-dark.svg" alt="Shadcn Labs" height="24">
  </picture>
</a>

localflow is backed by [Shadcn Labs](https://www.shadcn-labs.com/?utm_source=localflow&utm_medium=referral&utm_campaign=backed_by).
Want to sponsor it too? [DM me on X](https://x.com/radiumcoders) or
[sponsor on GitHub](https://github.com/sponsors/radiumcoders).

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

**Memory** (on Hyprland, where the dictation key also loads the cleanup model)

| State | RAM | GPU memory |
| --- | --- | --- |
| Idle (between dictations) | ~0.4 GB (Handy 0.3 + Ollama/proxy 0.08) | none |
| While dictating | ~1.4 GB | ~4.5 GB (cleanup model ~3.8 + speech model ~0.7) |
| Up to 2 min after the last dictation | ~1.5 GB | ~3.8 GB, then freed |

Pressing the dictation key runs `localflow --warm`, which loads the cleanup
model (~1.5 s) while you speak, so it's ready when you stop; it unloads 2
minutes after your last dictation. Handy's speech model loads in ~0.2 s and
unloads right after each dictation. Without a GPU, the cleanup model takes
~3.6 GB of RAM instead of GPU memory while loaded.

**Memory on macOS** with the local model and the Handy app setup
(`--handy-app`; MacBook Air M4, 16 GB; Activity Monitor's "Memory" column)

| State | Total | Breakdown |
| --- | --- | --- |
| Idle, after login | ~225 MB | Handy 52 + its WebKit helpers 67, Ollama app 69 + server 22, proxy 13 |
| Idle, after dictating | ~360 MB | Handy and its WebKit helpers grow to ~255 after first use |
| Up to 2 min after the last dictation | ~1.5 GB | + cleanup model 1.27 GB, then freed |

On macOS the proxy watches Handy's log and starts the warm-up the moment you
press `Option+Shift+Space`, so the model unloads 2 minutes after your last
dictation instead of staying loaded. On Apple Silicon the CPU and GPU share
memory, so the model file is mapped rather than copied (the default on
macOS): 1.27 GB instead of 4.8 GB for `qwen3.5:4b`, at the same speed.

qwen3.5 reads its ~2,800-token prompt slowly on Apple Silicon (~240
tokens/s), so the first dictation after the model unloads needs ~11–14 s
from the key press; whatever you say in that time hides it. After that, the
warm-up leaves a checkpoint of the prompt and a short dictation takes ~0.5 s.
Set a longer `LOCALFLOW_KEEP_ALIVE` (in `~/.config/localflow/env` for
`dictate`, or `~/Library/LaunchAgents/computer.localflow.plist` with
`--handy-app`) to trade memory for fewer slow starts.

On Windows (no warm-up yet) the model stays loaded for 1 hour after your last
dictation (`LOCALFLOW_KEEP_ALIVE` changes that). Cloud dictation apps use less
RAM because their models run on their servers; here everything runs on your
machine.

**Speed** (time from releasing the key to text appearing, after transcription)

| | Short sentence | Long ramble (~1 min of speech) |
| --- | --- | --- |
| GPU (RX 9070 XT) | ~0.4 s | ~1.5 s |
| CPU only (Ryzen 5 5600) | ~0.9 s | ~20 s |
| Apple M4 (MacBook Air), model loaded | ~0.5 s | ~5–7 s |

**Recommended:** 8 GB RAM minimum, 16 GB comfortable; a GPU with 4 GB+ of
VRAM (NVIDIA, AMD or Apple Silicon) for fast long dictation. Without a GPU,
`--model qwen3.5:2b` roughly halves the wait.

**Software:** Python 3.9+ with Tk (python.org and winget Pythons include
it; Homebrew: `brew install python-tk@3.x`; Arch: `tk`; Debian/Ubuntu:
`python3-tk`); the installers below need neither. On Linux also `curl` and `zstd` (to unpack Ollama, not needed with
`--api`), PipeWire (`pw-record`) or ALSA (`arecord`) for recording, Handy's
own needs (`webkit2gtk-4.1`), and for pasting `wl-clipboard` + `wtype` on
Wayland or `xclip` + `xdotool` on X11 (Omarchy ships all of these).

## Install

### Windows: the installer

Download `localflow-<version>-x64.msi` from
[Releases](https://github.com/radiumcoders/localflow/releases) and open it. It
needs no Python: it installs localflow to Program Files, adds it to the Start
menu and starts it when you sign in. Then open localflow from the Start menu:
the first time, it opens setup. Choose cleanup on this computer or with your
API key, and it downloads what that needs (Ollama and the model, or nothing;
Handy's speech engine and a speech model) with a progress bar. Open it from
the Start menu again to change these settings. Uninstall it from Settings >
Apps.

The installer isn't signed yet, so Windows SmartScreen warns about an
unknown publisher: click **More info**, then **Run anyway**.

For IT deployment, `msiexec /i localflow-<version>-x64.msi /qn`, then
`"C:\Program Files\localflow\localflow.exe" install --api anthropic` (with
`LOCALFLOW_API_KEY` set) sets it up without the window; it takes
`install.py`'s options.

### macOS: the disk image

Download `localflow-<version>-arm64.dmg` (Apple Silicon, M1 and later) or
`-x64.dmg` (Intel) from
[Releases](https://github.com/radiumcoders/localflow/releases), open it and
drag localflow into Applications. It needs no Python or Homebrew. Open it from
Applications: the first time, it opens setup, like on Windows, and from then
on it starts at login. When macOS asks, allow localflow under Accessibility
(the hotkey and pasting; it starts working as soon as you do) and Microphone.
Open it again to change the settings. To remove it, quit it with
`launchctl bootout gui/$(id -u)/computer.localflow.dictate`, then delete the
app and `~/Library/LaunchAgents/computer.localflow.dictate.plist`.

It isn't signed with an Apple Developer ID yet, so the first time macOS says
it can't check it for malware: open **System Settings > Privacy & Security**
and click **Open Anyway** next to localflow.

### Linux: the AppImage

Download `localflow-<version>-x86_64.AppImage` from
[Releases](https://github.com/radiumcoders/localflow/releases), make it
executable and run it:

```sh
chmod +x localflow-*-x86_64.AppImage
./localflow-*-x86_64.AppImage
```

It needs no Python, and a distro from 2023 or later (glibc 2.38+, for Handy's
speech engine: Ubuntu 24.04, Fedora 39, Debian 13, Arch). Setup copies localflow to `~/.local/share/localflow/app`
(your key bindings run it from there, so the AppImage can go anywhere,
or away), adds it to your app launcher, and shows the key bindings to add,
with a Copy button (the `bindings.lua` lines on Hyprland). Open localflow from
the launcher to change the settings. It uses your system's recording and
paste tools (see Software above).

### Any OS: `install.py`

Download just [`install.py`](https://raw.githubusercontent.com/radiumcoders/localflow/master/install.py)
and run it:

```sh
curl -O https://raw.githubusercontent.com/radiumcoders/localflow/master/install.py
python3 install.py
```

On Windows (PowerShell):

```powershell
irm https://raw.githubusercontent.com/radiumcoders/localflow/master/install.py -OutFile install.py
py install.py
```

On its own, `install.py` downloads the rest of localflow into
`~/.local/share/localflow` (Linux), `~/Library/Application Support/localflow`
(macOS) or `%LOCALAPPDATA%\localflow` (Windows) and runs it from there;
re-running it updates those files. Run from a `git clone`, it uses the clone.

`install.py` is safe to re-run. It:

1. installs Ollama if missing: Homebrew on macOS, winget on Windows, a per-user
   install in `~/.local/ollama` on Linux (no sudo; adds the ROCm build when an
   AMD GPU is found, `--no-rocm` to skip). With `--api` (below) it skips Ollama
2. pulls the cleanup model, **qwen3.5:4b** (~3.3 GB). On Apple Silicon Macs it
   pulls Ollama's MLX build, **qwen3.5:4b-mlx**, which runs faster there, and
   falls back to the regular one if that can't run. Optional: `--model
   qwen3.5:2b` for a faster, less accurate one
3. installs Handy if missing, for its speech engine: Homebrew cask or the
   `.dmg` on macOS, winget or the installer on Windows, the AppImage on Linux
   (x64 and arm64). It downloads a speech model if none is there: Handy's
   recommended one, or the one you name with `--speech-model` (see `handy
   --list-models`; e.g. `handy-computer/moonshine-base-gguf/moonshine-base-Q8_0.gguf`
   for a 73 MB one on machines without a GPU)
4. sets up [`dictate`](#dictating): on macOS and Windows it starts at login
   (launchd / the Startup folder); on Linux it prints the key bindings to add.
   On macOS it also installs `sox` for recording when you have Homebrew
   (without it, dictate records through macOS's own AudioQueue)

`--handy-app` sets up the older way instead: Handy's own app and hotkeys,
with a localflow proxy at login as Handy's post-processing step
(see "With the Handy app" below).

## Use your own API key (optional)

To use even less of your machine, the cleanup can run on a small hosted
model with your own key: **Claude Haiku** (the default), or any model behind
an OpenAI-compatible API. No Ollama and no local cleanup model: the proxy
itself takes ~25 MB of RAM and no GPU memory, and only Handy's speech model
runs locally. Measured with Haiku: ~0.5 GB RAM idle and while dictating
(almost all of it Handy and its WebKit window processes) plus ~0.7 GB GPU
memory while dictating, versus ~1.9 GB RAM + ~4.5 GB GPU memory with the
local model. Haiku scored 34/36 on `cases.txt`
(local `qwen3.5:4b`: 33/36), at ~1.3 s per dictation instead of ~0.4 s.

The trade-off: **every dictation is sent to that provider**, and it costs a
fraction of a cent (Haiku 5.5: $0.10 / $0.50 per million input / output
tokens; each dictation sends ~3,700 tokens of rules and examples, cached
between dictations, so under $0.0005 each).

```sh
python3 install.py --api anthropic                     # always claude-haiku-5-5
python3 install.py --api openai --model <model>         # any other provider: name its model
python3 install.py --api https://my-server.example/v1 --model <model>  # any OpenAI-compatible URL
```

`--api` takes `anthropic`, `openai`, `openrouter`, `groq`, `gemini`,
`mistral`, `deepseek`, or a base URL; `--model` is the provider's model name.
With `anthropic` the model is always Claude Haiku 5.5: `--model` and
`LOCALFLOW_MODEL` can't change it. Claude goes through Anthropic's own Messages API,
with thinking turned off and the rules and examples cached between
dictations; everything else through the OpenAI-compatible chat API.
The installer asks for the key once (or reads `LOCALFLOW_API_KEY`), saves it
to `~/.config/localflow/api_key` (`%APPDATA%\localflow\api_key` on Windows),
readable only by you and kept out of the service file, and checks the model
answers before switching over. Delete that file and re-run to change the key.
Re-running `install.py` without `--api` switches back to local; on Linux,
`systemctl --user disable --now ollama` frees Ollama's memory while you're on
an API.

For other providers, pick their smallest, fast **non-reasoning** model:
thinking models add seconds per dictation (reasoning models that refuse
`temperature`/`max_tokens`, like OpenAI's, still work, just slower).
Providers retire small models often (Groq shut down `llama-3.1-8b-instant`
in August 2026), so check their model list. If the API is down, the key is
wrong or you're offline, you get the raw transcript, as with Ollama. The
rules and checks were tuned on `qwen3.5:4b`; to see how a hosted model does,
run `LOCALFLOW_API_URL=anthropic ./localflow --eval`.

## Dictating

`dictate` puts a small pill at the bottom of the screen while you talk: a red
dot and a live level meter, then "Transcribing…" and "Cleaning up…". The
text is pasted into the window you were typing in; the pill never takes
focus.

| Platform | Hotkey |
| --- | --- |
| macOS | hold `Option+Shift+Space` and talk, or tap it, talk, tap again |
| Windows | hold `Ctrl+Shift+Space` and talk, or tap it, talk, tap again |
| Linux | keys you bind to `dictate start` / `dictate stop` (below) |

`Esc` cancels on macOS and Windows; `dictate cancel` on Linux.

Under the hood it records the microphone (winmm on Windows, AudioQueue or `sox` on macOS,
`pw-record` on Linux), transcribes with Handy's headless `handy
--transcribe-file` (the same engine and speech model as Handy's app, loaded
only for the ~0.5 s it runs), cleans up with `localflow` and pastes. Handy's
app itself doesn't run. Settings come from `~/.config/localflow/env`
(`%APPDATA%\localflow\env` on Windows), which `install.py` writes; change the
speech model in Handy's settings (open Handy, pick, quit). Logs (never your
words) go to `~/.local/state/localflow/dictate.log`,
`~/Library/Logs/localflow/dictate.log` or `%LOCALAPPDATA%\localflow\dictate.log`.

Measured on Linux with Claude Haiku and Handy's Parakeet 0.6B speech model:

| | Handy app + proxy + Ollama | `dictate` |
| --- | --- | --- |
| Idle | ~470 MB RAM, 23 MB GPU | **0** on Linux; ~25 MB on macOS / Windows (waits for the hotkey) |
| While you talk | ~510 MB RAM + 730 MB GPU | **~31 MB** RAM (overlay + recorder) |
| Transcribing (~0.5 s) | | ~185 MB RAM + 830 MB GPU |
| For 3 min after a paste (learning your fixes) | | ~25 MB |
| Key release to text | ~1.5 s | ~2–2.5 s |

`dictate --selftest` checks the overlay (and that it leaves focus alone), the
clipboard, the hotkey, the microphone and Handy.

**macOS:** allow the Python that runs `dictate` (`install.py` prints its
path) in System Settings > Privacy & Security > **Accessibility**, for the
hotkey and the paste, and **Microphone** when asked. Without Tk
(`brew install python-tk@3.x` for Homebrew's Python), it shows notifications
instead of the pill.

**Windows:** Python from python.org or winget includes Tk. `dictate` runs
hidden through `pythonw`; to restart it, end `pythonw.exe` in Task Manager
and sign in again (or run `pythonw dictate`).

**Without a GPU**, pick a small speech model in Handy (Moonshine Base peaks
at ~210 MB RAM on CPU, Parakeet 0.6B at ~1.2 GB).

### Referring to files

In a coding app, say the file: "can you refer to the install file and check
why it fails on Mac" pastes "Can you refer to @install.py and check why it
fails on Mac?", with the reference put in the way the app takes it:

| You're dictating into | A file you name becomes |
| --- | --- |
| Cursor, VS Code, Windsurf | `@path`, relative to the project (Cursor's agent and Claude Code read the file from it) |
| A terminal (Claude Code, Codex, opencode…) | `@path`, relative to where the terminal is |
| Anything else (a browser, Slack, mail) | the file itself, pasted as an attachment |

While you talk, `dictate` works out the project: the folder open in the
editor window (from its title and the editor's list of open folders), or the
terminal's working directory (from its title, else the coding agent or newest
program running in it). Elsewhere, it's the project you last had open in your
editor. When what you said mentions a file ("file", "folder", "script", "dot
py", "read me", "refer", "attach"…), the project's best-matching paths go to
the cleanup with your words, and it writes `@path` for the ones you meant
("install the package" stays as said). Only those paths are sent, never file
contents; raw dictation skips all this.

Tested on Linux with Cursor (its Claude Code panel), Ghostty and Chrome. On
Windows, a terminal's directory can't be read, so terminals there use the
editor's project. On macOS, the window title needs the Accessibility
permission, which `dictate` already asks for.

### Learning from your fixes

When `dictate` gets a word wrong and you fix it, it learns the fix on its
own. For 3 minutes after each paste (or until your next dictation), it looks
for the version you kept:

| Where you dictated | How it sees your fix | Linux | macOS | Windows |
| --- | --- | --- | --- | --- |
| Claude Code (terminal or editor panel), Codex | the message you sent, from its transcript | ✅ | ✅ | ✅ |
| Cursor's chat | the message you sent, from Cursor's chat history | ✅ | ✅ | ✅ |
| Any other text box | the box itself, once you send it or move on | – | ✅ | ✅ |

Linux apps don't expose their text boxes (Chrome and Electron only do with a
startup flag), so there it's the first two rows, plus the learn key.

It compares what it pasted with what you kept. The words you changed go to
the cleanup model, which keeps fixes of misheard words ("Claw" to "Claude",
"omar key" to "Omarchy", "Sara" to "Sarah") and drops changes of mind
("Tuesday" to "Wednesday", "Hilton" to "Marriott"). Grammar fixes between
everyday words ("there" to "their") and rewritten sentences never count.
Each fix is saved to your vocabulary, `~/.config/localflow/words.txt`
(`%APPDATA%\localflow\words.txt` on Windows), as `Claude: often heard as
"Claw"`, and from then on the cleanup writes Claude. A notification (the pill
on macOS and Windows) says what it learned. The file is plain text: edit or
delete lines to unlearn.

For anywhere else, or to teach it right away, select the fixed sentence (or
copy it) and press the learn key, which saves your fixes without asking the
model:

| Platform | Learn key |
| --- | --- |
| macOS | `Option+Shift+L` |
| Windows | `Ctrl+Alt+Space` |
| Linux | a key you bind to `dictate learn` (F8 below) |

`LOCALFLOW_LEARN=key` in `~/.config/localflow/env` turns off learning on its
own (just the key); `off` turns off both. What it reads stays on your
machine except the changed sentence, which goes to your cleanup model (Haiku
or Ollama) once, for the check. Nothing is logged. To compare, the last
pasted text is kept in a temporary folder only you can read, until the next
dictation replaces it. While it watches, it uses ~25 MB (a background process
on Linux, a thread in the macOS / Windows app).

### Linux (Hyprland / Omarchy)

Wayland apps can't grab global keys, so the compositor owns them, and nothing
runs between dictations. In `~/.config/hypr/bindings.lua`:

```lua
hl.unbind("SUPER + CTRL + X")
hl.unbind("F9")
o.bind("SUPER + CTRL + X", "Toggle dictation", "~/Projects/localflow/dictate toggle")
o.bind("F9", "Start dictation (push-to-talk)", "~/Projects/localflow/dictate start")
o.bind("F9", "Stop dictation (push-to-talk)", "~/Projects/localflow/dictate stop", { release = true })
o.bind("F10", "Start raw dictation (push-to-talk)", "~/Projects/localflow/dictate start raw")
o.bind("F10", "Stop raw dictation (push-to-talk)", "~/Projects/localflow/dictate stop", { release = true })
o.bind("F8", "Learn from a dictation correction", "~/Projects/localflow/dictate learn")
```

The pill needs Tk (`sudo pacman -S tk`); without it you get a notification.
On Hyprland, `dictate` adds a window rule at runtime that rounds the pill and
keeps it at the bottom of the focused monitor. It pastes with `./paste`:
Ctrl+V in apps, Shift+Insert in terminals, like Omarchy's universal paste,
and in VS Code-family editors (their chat panels, like Claude Code's in
Cursor, ignore a synthetic Ctrl+V), restoring your clipboard after.

On other desktops, add custom shortcuts running `dictate toggle` (or
`dictate start` on press and `dictate stop` on release). Pasting needs
`wl-clipboard` + `wtype` on Wayland, or `xclip` (or `xsel`) + `xdotool` on
X11. Where the paste keys can't be sent (GNOME's Wayland session doesn't
allow `wtype`), the text stays on the clipboard and the pill says so: press
Ctrl+V.

### With the Handy app (`install.py --handy-app`)

Wayland apps can't grab global shortcuts, so let the compositor own the keys.
On Hyprland, `install.py` moves Handy's own shortcuts out of the way (they grab
Ctrl+Space and start stray recordings) and sets Handy to paste through
`./paste`: Ctrl+V in apps, Shift+Insert in terminals, like Omarchy's universal
paste, restoring your clipboard after. In `~/.config/hypr/bindings.lua`:

```lua
hl.unbind("SUPER + CTRL + X")
hl.unbind("F9")
-- `localflow --warm` loads the cleanup model while you speak (see Memory)
o.bind("SUPER + CTRL + X", "Toggle dictation", "~/.local/bin/handy --toggle-post-process & ~/Projects/localflow/localflow --warm")
o.bind("F9", "Start dictation (push-to-talk)", "~/.local/bin/handy --toggle-post-process & ~/Projects/localflow/localflow --warm")
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

Your own list, `~/.config/localflow/words.txt`, is read too: the learn key
adds to it (see [Learning from your fixes](#learning-from-your-fixes)), and
you can edit it. Both are read on every dictation; no restart needed.

## Tuning

```sh
echo "call mom tomorrow actually tonight" | ./localflow
./localflow --eval                 # run cases.txt against the current model
./localflow --eval qwen3.5:2b      # compare another Ollama model
./localflow --eval qwen3.5:4b off  # draft only (no repair call)
./localflow --check                # is the model reachable?
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

- Moving a cloned project folder breaks `dictate` at login, your key bindings and
  the proxy service (they point at it); re-run `install.py` from the new place.
- On Linux, Handy runs from an extracted AppImage, so Handy's in-app updates
  don't apply. To update: delete `~/.local/share/handy` and re-run `install.py`.
- Tested by hand on Linux (Arch/Omarchy, Hyprland), and the Handy app setup
  on macOS (MacBook Air M4). CI tests every push on fresh Ubuntu, macOS and
  Windows machines: the unit tests, `dictate --selftest`, and `tests/e2e.py`,
  which runs `install.py`, speaks a sentence with the OS's text-to-speech and
  dictates it into a text box through the hotkey, the overlay, Handy's
  transcription, the cleanup and the paste. CI machines have no microphone,
  so recording (AudioQueue or sox on macOS, winmm on Windows) is the one piece
  only a person can check there: `dictate --selftest` on your machine does.
  The same CI also builds the Windows `.msi`, the macOS `.dmg` and the Linux
  AppImage, installs each on a fresh machine, sets it up and self-tests the
  installed app (`tests/msi.py`, `tests/dmg.py`, `tests/appimage.py`).
- `dictate` needs a Handy version with the headless `--transcribe-file` option
  (check with `handy --help`).
- On Hyprland, apps running through XWayland (Tk's own windows, some older
  apps) get garbled keys from `wtype`, so pasting into them fails; native
  Wayland apps (browsers, Cursor, terminals) are fine.
- File references are found by name: "the install file" finds `install.py`,
  but a description ("the file with the hotkey code") doesn't. On Windows, a
  terminal's own folder can't be read (see [Referring to files](#referring-to-files)).
- Misheard words are only fixed reliably when they're in `words.txt` with a
  hint; from context alone a 4b model often can't tell (Javanese vs Japanese).

## Files

| File | What |
| --- | --- |
| `localflow` | rules + examples, `serve` proxy, stdin filter, `--eval` |
| `install.py` | cross-platform installer |
| `words.txt` | your names/terms, with optional mishearing hints |
| `cases.txt` | eval cases |
| `dictate` | the dictation app: overlay, hotkey, recording, transcription, cleanup, paste |
| `tests/` | `python -m unittest discover -s tests` (no key, model or microphone needed); `e2e.py`, the install-and-dictate test CI runs; `msi.py`, `dmg.py` and `appimage.py`, the installer tests |
| `paste` | Linux/Hyprland paste helper for Handy and `dictate` |
| `assets/` | images for this README |
| `pkg/` | the packaged app: `build.py` (PyInstaller, then the `.msi` with WiX, the `.dmg` or the AppImage), its setup window and entry point |
