# localflow

Talk, and get clean text where you type. A native dictation app for macOS,
Windows and Linux, written in Rust: its own speech engine (whisper.cpp,
in-process), its own small language model, `localflow-clean`, that turns what
you said into what you meant, and a pill at the bottom of the screen while you
talk. Nothing leaves your machine. English only.

| You say | You get |
| --- | --- |
| "let's meet in 3 hours wait in 2 hours" | Let's meet in 2 hours. |
| "um I think we should uh ship it on Monday" | I think we should ship it on Monday. |
| "my tasks today are first buy milk second call the bank third fix the login bug" | My tasks today are:<br>1. Buy milk<br>2. Call the bank<br>3. Fix the login bug |
| "send the deck to Priya *(mutters: yeah one sec I'm on a call)* and ask her to check the pricing slide" | Send the deck to Priya and ask her to check the pricing slide. |
| "generate some code using Claw and push it to GitHub" | …using Claude… (from your `words.txt`) |

It handles self-corrections ("wait", "actually", "sorry I mean", "scratch that",
"I just remembered"), filler words, punctuation, numbered and nested lists in
long rambling speech, paragraphs and `## ` section headings for long
multi-topic dictation, spoken commands ("new paragraph", "new line",
"heading …", "bullet point …"), misheard names, and
[asides](#asides-talking-to-yourself): what you say to yourself or to someone
else while dictating is left out. It never answers what you say: a dictated
question stays a question.

For coding, files you name become `@path` references
([Referring to files](#referring-to-files)), and in a terminal running Claude
Code, Codex and the like, "…and send it" sends the prompt while "stop",
"undo that" and "clear the chat" press the keys for you
([Talking to a coding agent](#talking-to-a-coding-agent)).

## Install

### Windows

Download `localflow-<version>-x64.msi` from
[Releases](https://github.com/radiumcoders/localflow/releases) and open it. It
installs localflow to Program Files, adds it to the Start menu and starts it
when you sign in. Open localflow from the Start menu: the first time, the
onboarding opens. Pick a speech model (it downloads while you go on), and
localflow downloads its own cleanup model (351 MB) too and runs it itself:
there's nothing else to install. Then try a dictation in the window. Open it
from the Start menu again to change the settings. Uninstall it from Settings > Apps;
that leaves your settings and vocabulary in `%APPDATA%\localflow` and the
models in `%LOCALAPPDATA%\localflow`, which you can delete.

The installer isn't signed yet, so SmartScreen warns about an unknown
publisher: click **More info**, then **Run anyway**.

For IT deployment: `msiexec /i localflow-<version>-x64.msi /qn` installs it
for everyone on the computer; then, as each user,
`"C:\Program Files\localflow\localflow.exe" setup` downloads their models
(see [`localflow setup`](#settings)). An upgrade closes the running localflow
and starts the new one.

### macOS

Download `localflow-<version>-arm64.dmg` (Apple Silicon) or `-x64.dmg` (Intel)
from [Releases](https://github.com/radiumcoders/localflow/releases), open it
and drag localflow into Applications. Open it from Applications: the
onboarding opens and adds localflow as a login item. It asks you to allow
localflow under Accessibility (the hotkey and pasting) and Microphone. Keep
it in Applications: the login item runs it from there.

To remove it: `localflow quit` (or
`launchctl bootout gui/$(id -u)/computer.localflow.dictate`), then delete the
app, `~/Library/LaunchAgents/computer.localflow.dictate.plist`, and if you
like `~/Library/Application Support/localflow` (the models),
`~/.config/localflow` (settings, vocabulary, history) and
`~/Library/Logs/localflow`. The app isn't signed with a Developer ID yet, so
to macOS each update is a new app: if the hotkey stops working after one,
switch localflow off and on again under Privacy & Security > Accessibility.

It isn't signed with an Apple Developer ID yet, so the first time macOS says
it can't check it: open **System Settings > Privacy & Security** and click
**Open Anyway** next to localflow.

### Linux

Download `localflow-<version>-x86_64.AppImage` from
[Releases](https://github.com/radiumcoders/localflow/releases), make it
executable and run it:

```sh
chmod +x localflow-*-x86_64.AppImage
./localflow-*-x86_64.AppImage
```

The onboarding opens. Once you've picked a speech model, it copies
localflow to `~/.local/share/localflow/app` (your key bindings run it from
there, so the AppImage can go anywhere, or away) and adds it to your app
launcher; then it shows the key bindings to add, with a Copy button, and lets
you try a dictation. `localflow onboarding` opens it again; running a newer
AppImage updates the copy. It needs glibc 2.39+ (Ubuntu 24.04, Fedora 40,
Debian 13, Arch), the Vulkan loader (`libvulkan1` / `vulkan-icd-loader`, on
every desktop with a GPU driver), PipeWire (with `pipewire-alsa`) or ALSA for
recording, and for pasting `wl-clipboard` + `wtype` on Wayland or `xclip` +
`xdotool` on X11 (Omarchy ships all of these). Transcription and cleanup run
on the GPU through Vulkan when there is one (Mesa's `vulkan-radeon` /
`vulkan-intel`, or NVIDIA's driver), else on the CPU. Without FUSE, run it
with `APPIMAGE_EXTRACT_AND_RUN=1`.

### From source

```sh
git submodule update --init --depth 1   # llama.cpp, for the cleanup model
cargo build --release          # needs Rust and cmake (whisper.cpp, llama.cpp); on Linux
                               # the ALSA, Wayland, X11, xkbcommon and fontconfig headers
target/release/localflow       # the settings window (Linux) or the background app
cargo xtask package --version 0.2.0 [--features vulkan]   # the installer, in build/
```

## Dictating

| Platform | Hotkey |
| --- | --- |
| macOS | hold `Option+Shift+Space` and talk, or tap it, talk, tap again |
| Windows | hold `Ctrl+Shift+Space` and talk, or tap it, talk, tap again |
| Linux | keys you bind to `localflow start` / `localflow stop` (below) |

`Esc` cancels on macOS and Windows; `localflow cancel` on Linux.

A pill at the bottom of the screen shows a red dot and your microphone's level
while you talk, then "Transcribing…" and "Cleaning up…". The text is pasted
into the window you were typing in; the pill never takes focus (on Wayland
it's a layer-shell surface, so no window rules are needed).

### Linux (Hyprland / Omarchy)

Wayland apps can't grab global keys, so the compositor owns them, and nothing
runs between dictations. In `~/.config/hypr/bindings.lua` (the settings window
shows these with your path, and a Copy button):

```lua
hl.unbind("SUPER + CTRL + X")
hl.unbind("F9")
o.bind("SUPER + CTRL + X", "Toggle dictation", "~/.local/share/localflow/app/localflow toggle")
o.bind("F9", "Start dictation (push-to-talk)", "~/.local/share/localflow/app/localflow start")
o.bind("F9", "Stop dictation (push-to-talk)", "~/.local/share/localflow/app/localflow stop", { release = true })
o.bind("F10", "Start raw dictation (push-to-talk)", "~/.local/share/localflow/app/localflow start raw")
o.bind("F10", "Stop raw dictation (push-to-talk)", "~/.local/share/localflow/app/localflow stop", { release = true })
o.bind("F8", "Learn from a dictation correction", "~/.local/share/localflow/app/localflow learn")
```

Raw dictation pastes the transcript as heard, without cleanup. On other
desktops, add custom shortcuts running `localflow toggle` (or `start` on
press and `stop` on release). It pastes with Ctrl+V in apps, Shift+Insert in
terminals and VS Code-family editors (their chat panels ignore a synthetic
Ctrl+V), restoring your clipboard after. Where the paste keys can't be sent
(GNOME's Wayland session doesn't allow `wtype`), the text stays on the
clipboard and the pill says so: press Ctrl+V.

## How it works

Each dictation is a short-lived process: it starts recording at once, shows
the pill, and exits once it has pasted. On macOS and Windows a background app
of ~6 MB waits for the hotkey and starts it; on Linux your key bindings do.

1. **Speech to text**: whisper.cpp runs inside localflow. The speech model
   loads only while it transcribes (40–200 ms to load) and is dropped right
   after. Silero voice detection skips silence first, and whisper encodes a
   window as long as your dictation instead of 30 s, which makes a short
   dictation several times faster. Your vocabulary (`words.txt`) primes the
   spelling of names (on Small and larger models). Each segment's loudness is
   measured, for [asides](#asides-talking-to-yourself).
2. **Cleanup**: localflow's own model, `localflow-clean`, cleans and formats
   the transcript, with the spellings in your `words.txt`. It learned the
   rules from examples, so it gets no long prompt: it loads while you speak
   and answers in ~0.06 s on a GPU.

If the cleanup model can't run or answers with something much longer than you
said, you get the transcript as heard. You never lose a dictation.

## Asides: talking to yourself

While dictating you think aloud ("hmm, where was I", "let me think"), answer
someone in the room ("one sec, I'm on a call"), or mutter. None of that is
meant for the text, and localflow leaves it out:

- **Muttered**: the speech engine measures each segment's loudness; anything
  9 dB or more below your main voice (muttering, someone in the background) is
  marked quiet and left out unless it clearly continues your sentence.
- **Said to yourself or someone else**: the cleanup recognises thinking aloud,
  remarks to people around you and comments about the dictation ("is this
  recording") and leaves them out. When unsure, it keeps your words.
- If a dictation was nothing but an aside, nothing is pasted ("Left out an
  aside").

## Requirements & resource use

Measured on Linux (Ryzen 5 5600, 6 cores; Radeon RX 9070 XT) with the default
speech model, Small (English):

| State | Memory |
| --- | --- |
| Between dictations | **nothing** on Linux; **~6 MB** on macOS / Windows (the hotkey listener) |
| While you talk | ~40 MB of its own (the pill and the recorder; the GPU libraries it shares with other apps not counted) |
| Transcribing (~0.1–0.8 s) | ~100 MB RAM + 450 MB GPU memory with Vulkan / Metal, ~330 MB RAM on the CPU |

| 9 s of speech | RAM | GPU memory | Time |
| --- | --- | --- | --- |
| tiny.en, CPU | 104 MB | – | 0.18 s |
| base.en, CPU | 164 MB | – | 0.27 s |
| **small.en, CPU** | 330 MB | – | 0.78 s |
| **small.en, Vulkan** | ~100 MB | 448 MB | 0.11–0.21 s |
| large-v3-turbo, Vulkan | 118 MB | 846 MB | 0.26–0.51 s |
| (before: Handy's Parakeet, Vulkan) | 183 MB | 752 MB | 0.16–0.45 s |

Small heard the same words as Handy's Parakeet on real recordings, at about
half the memory. On a machine without a GPU, Base (82 MB) is faster still.

Cleanup on this computer runs localflow's own model, `localflow-clean`: Qwen3.5
0.8B fine-tuned on the cleanup rules alone, with an English-only vocabulary, in
a 351 MB file. localflow runs it in its own process with llama.cpp (Vulkan,
Metal, or the CPU), loads it while you speak (~0.1 s) and frees it after the
dictation, so it uses no memory between dictations. While dictating, speech and
cleanup together peak at ~460 MB.

| Cleanup | Under 30 words | 100+ words | cases.txt | Memory |
| --- | --- | --- | --- | --- |
| localflow-clean, GPU (RX 9070 XT) | ~0.06 s | ~0.34 s | 32/41 | ~530 MB GPU, freed after |
| localflow-clean, CPU only | ~0.25 s | ~1.8 s | 32/41 | ~510 MB, freed after |
| (before: qwen3.5:4b in Ollama, GPU) | ~0.33 s | ~1.8 s | 35/41 | ~4.3 GB GPU, kept loaded |
| (before: Claude Haiku with your API key) | ~1.0–1.3 s | ~2 s | 41/41 | – |

On 142 held-out examples written to the rules, localflow-clean matched the
expected text exactly in 51% (qwen3.5:4b: 27%).

## Referring to files

In a coding app, say the file: "can you refer to the install file and check
why it fails on Mac" pastes "Can you refer to @install.py and check why it
fails on Mac?", with the reference put in the way the app takes it:

| You're dictating into | A file you name becomes |
| --- | --- |
| Cursor, VS Code, Windsurf | `@path`, relative to the project |
| A terminal (Claude Code, Codex, opencode…) | `@path`, relative to where the terminal is |
| Anything else (a browser, Slack, mail) | the file itself, pasted as an attachment |

While you talk, localflow works out the project: the folder open in the
editor window (from its title and the editor's list of open folders), or the
terminal's working directory (from its title, else the coding agent or newest
program running in it). Elsewhere, it's the project you last had open in your
editor. When what you said mentions a file ("file", "folder", "script", "dot
py", "read me", "refer", "attach"…), the project's best-matching paths go to
the cleanup with your words, and it writes `@path` for the ones you meant.
Only those paths are sent, never file contents; raw dictation skips all this.
On Windows, a terminal's own folder can't be read, so terminals there use the
editor's project.

## Talking to a coding agent

When the terminal you dictate into runs a coding agent (Claude Code, Codex,
opencode, Gemini CLI, aider, Amp, Goose…), localflow notices it (from the
programs running in that window) and a few things change:

| You say | It does |
| --- | --- |
| "…and send it" at the end ("send it", "submit it", "and send") | pastes the prompt without those words, then presses Enter |
| just "stop" ("cancel", "hold on", "wait") | Esc: interrupts the agent |
| just "undo that" ("go back", "rewind") | Esc Esc: Claude Code's rewind |
| just "clear the chat" ("start over", "new chat") | `/clear` (`/new` in Codex and opencode) |
| just "compact" ("compact the context") | `/compact` (`/compress` in Gemini CLI) |
| just "send it" | Enter: sends what's in the prompt already |

A command only counts when it's the whole dictation: "stop using the old API"
is a prompt. Without "send it", the prompt is pasted and left for you to read
over (and fix: the learn key learns from that). "Send prompts to coding agents"
in the settings (`LOCALFLOW_SUBMIT`): `said` (default), `always`, `never`.
Anywhere else, what you say is pasted as always and nothing is pressed.

## History

Every dictation is kept on your computer: when, where it went, what was heard
and what went out (pasted, sent, or a command's keys). The last 200, readable
only by you, next to the log; "Keep a history" in the settings
(`LOCALFLOW_HISTORY=off`) turns it off.

```sh
localflow history       # the last 10 (or: localflow history 50)
localflow history --last | wl-copy   # the last text again, e.g. when a paste went astray
localflow history --clear
```

## Learning from your fixes

When localflow gets a word wrong, fix it, select the fixed sentence (or copy
it) and press the learn key: `Option+Shift+L` (macOS), `Ctrl+Alt+Space`
(Windows), or a key bound to `localflow learn` (Linux). The words you changed
("Claw" to "Claude", "omar key" to "Omarchy") are saved to your vocabulary,
`~/.config/localflow/words.txt` (`%APPDATA%\localflow\words.txt` on Windows),
as `Claude: often heard as "Claw"`, and from then on it's spelled right. It
compares your fix with the last dictation, which is kept on your machine,
readable only by you. Nothing is logged.

## Your vocabulary

`words.txt` lists names and terms you say, one per line, optionally with a hint
about how they get misheard:

```
Claude: the AI assistant, often heard as "Claw" or "clawed"
Pine Script: TradingView's scripting language, often heard as "Py script"
```

Your own list, `~/.config/localflow/words.txt`, is read too: learning adds to
it, and you can edit it. Both are read on every dictation.

## Settings

The settings window writes `~/.config/localflow/env` (`%APPDATA%\localflow\env`
on Windows); a variable set in the environment wins. `localflow setup` does the
same without the window (`--model`, `--speech-model`), downloads the speech
and cleanup models and puts the app in place.

| Variable | What |
| --- | --- |
| `LOCALFLOW_SPEECH_MODEL` | `tiny.en`, `base.en`, `small.en` (default), `large-v3-turbo` (see `localflow models`) |
| `LOCALFLOW_SPEECH_GPU` | `off` to transcribe on the CPU (`on` to try the GPU on an Intel Mac, where the CPU is the default) |
| `LOCALFLOW_MODEL` | the cleanup model: `localflow-clean` (default) or a `.gguf` file |
| `LOCALFLOW_CLEANUP_GPU` | `off` to clean up on the CPU (the default on an Intel Mac) |
| `LOCALFLOW_SUBMIT` | in a coding agent, press Enter after pasting: `said` (when you end with "send it", default), `always`, `never` |
| `LOCALFLOW_HISTORY` | `off` to keep no history of your dictations |

Logs (never your words; those are in the [history](#history)) go to `~/.local/state/localflow/dictate.log`,
`~/Library/Logs/localflow/dictate.log` or `%LOCALAPPDATA%\localflow\dictate.log`.

## Tools and tuning

```sh
localflow models [download ID]       # speech models and the cleanup model (localflow-clean)
localflow history [N]                # what you said and what went out
localflow listen 5                   # record 5 s, transcribe, clean up, print (segments and levels on stderr)
localflow transcribe talk.wav        # the same from a WAV file
echo "call mom tomorrow actually tonight" | localflow -
localflow --check                    # does the cleanup model load?
localflow --eval                     # cases.txt against the cleanup model
localflow --selftest                 # each piece that doesn't need a person
localflow quit                       # stop the background app (macOS, Windows)
```

The model is trained in a repository of its own (localflow-model);
`crates/core/src/local.rs` has the prompt it was trained with. `cases.txt`
holds `raw => expected` cases, compared ignoring case and punctuation (list
cases compare the numbered items; `? +must; -must not; order: a < b` checks
properties of long dictation). After a new model, run `localflow --eval`.

## Testing

`cargo test --workspace` needs no model, microphone or network. CI
(`.github/workflows/ci.yml`) runs it, clippy, and `cargo xtask e2e` on fresh
Linux, macOS and Windows machines: it speaks a sentence with the OS's
text-to-speech and dictates it, through the hotkey (Linux: `start`/`stop`),
the pill, our speech engine, the cleanup model and the paste, into a
text box in another process; only the microphone is replaced by the WAV. The
package workflow builds the `.msi`, both `.dmg`s and the AppImage, and
`cargo xtask installed` installs each on a fresh machine, checks it, dictates
with it and removes it. CI machines have no microphone, so recording itself is
what `localflow --selftest` checks on yours.

## Known limitations

- The installers aren't signed yet (SmartScreen and Gatekeeper ask once).
- On Hyprland, apps running through XWayland get garbled keys from `wtype`,
  so pasting into them fails; native Wayland apps are fine.
- File references are found by name: "the install file" finds `install.py`,
  but a description ("the file with the hotkey code") doesn't.
- Misheard words are fixed reliably only when they're in `words.txt` with a
  hint.
- Windows transcribes on the CPU (the GPU build needs the Vulkan SDK; Small
  takes ~1 s there). Intel Macs use the CPU too, by default: Metal on their
  integrated GPUs is slower than the CPU, or crashes.
- On Windows, the hotkey and the paste don't reach windows running as
  administrator (an elevated terminal, Task Manager): nothing happens there.
  Ctrl+Shift+Space is taken from other programs while localflow runs.
- Only text on the clipboard is put back after a paste; a copied image or
  file is gone.
- On Wayland desktops other than Hyprland, localflow can't see which window
  has focus, so terminals there get Ctrl+V (not Shift+Insert) and coding
  agents aren't recognised.

## Layout

| Path | What |
| --- | --- |
| `crates/core` | the cleanup with localflow's own model; your vocabulary and learning |
| `crates/llm` | llama.cpp in a library of its own, embedded in the binary, that runs the cleanup model |
| `crates/speech` | the microphone, the speech engine (whisper.cpp), its models, quiet segments |
| `crates/desktop` | where you're dictating, paste and clipboard, hotkeys, the learn key |
| `crates/app` | `localflow`: the GPUI pill and settings window, sessions, setup, the CLI |
| `crates/xtask` | `cargo xtask package / e2e / installed` |
| `crates/testbox` | the text box the end-to-end test dictates into |
| `pkg/localflow.wxs` | the Windows installer |
| `words.txt`, `cases.txt` | the shipped vocabulary, the eval cases |
