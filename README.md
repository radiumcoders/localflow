# localflow

Talk, and get clean text where you type. A native dictation app for macOS,
Windows and Linux, written in Rust: its own speech engine (whisper.cpp,
in-process), a small LLM that turns what you said into what you meant, and a
pill at the bottom of the screen while you talk. Nothing leaves your machine
unless you opt into [your own API key](#use-your-own-api-key-optional).

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
onboarding opens. Pick a speech model (it downloads while you go on) and where
cleanup runs (on this computer with Ollama, or with your API key), then try a
dictation in the window. Open it from the Start menu again to change the
settings. Uninstall it from Settings > Apps.

The installer isn't signed yet, so SmartScreen warns about an unknown
publisher: click **More info**, then **Run anyway**.

For IT deployment: `msiexec /i localflow-<version>-x64.msi /qn`, then
`"C:\Program Files\localflow\localflow.exe" setup --api anthropic` with
`LOCALFLOW_API_KEY` set (see [`localflow setup`](#settings)).

### macOS

Download `localflow-<version>-arm64.dmg` (Apple Silicon) or `-x64.dmg` (Intel)
from [Releases](https://github.com/radiumcoders/localflow/releases), open it
and drag localflow into Applications. Open it from Applications: the
onboarding opens and adds localflow as a login item. It asks you to allow
localflow under Accessibility (the hotkey and pasting) and Microphone. To
remove it: `launchctl bootout gui/$(id -u)/computer.localflow.dictate`, then
delete the app and `~/Library/LaunchAgents/computer.localflow.dictate.plist`.

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

The onboarding opens. Once you've picked where cleanup runs, it copies
localflow to `~/.local/share/localflow/app` (your key bindings run it from
there, so the AppImage can go anywhere, or away) and adds it to your app
launcher; then it shows the key bindings to add, with a Copy button, and lets
you try a dictation. `localflow onboarding` opens it again. It needs glibc 2.39+
(Ubuntu 24.04, Fedora 40, Debian 13, Arch), PipeWire or ALSA for recording,
and for pasting `wl-clipboard` + `wtype` on Wayland or `xclip` + `xdotool` on
X11 (Omarchy ships all of these). Transcription runs on the GPU through Vulkan
when there is one.

### From source

```sh
cargo build --release          # needs Rust and cmake (whisper.cpp); on Linux the
                               # ALSA, Wayland, X11, xkbcommon and fontconfig headers
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
2. **Draft**: the cleanup model cleans and formats the transcript, guided by
   the rules, worked examples and your `words.txt`.
3. **Review**: automatic checks look for concrete mistakes in the draft: a
   `words.txt` term still misheard ("Py script" left instead of Pine Script),
   a retraction left in ("pretty good, actually this is not pretty good, this
   is awesome"), or a qualifier you said ("maybe", "only", "never") that got
   dropped. If they find something, a fresh call (no examples, no history)
   repairs just that, and the repair is kept only if it clears the problem
   without adding new ones. Otherwise the draft goes out untouched, at no
   extra cost.

If the cleanup model is down or answers with something much longer than you
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

On `cases.txt` with Claude Haiku: 41/41 with asides left out (including two
look-alikes that must stay, like "tell Sarah I'm on a call and I'll ring her
back at 3"), 37/41 with them kept. Turn it off in the settings ("Leave out
asides") or with `LOCALFLOW_ASIDES=keep`.

## Requirements & resource use

Measured on Linux (Ryzen 5 5600, 6 cores; Radeon RX 9070 XT) with the default
speech model, Small (English):

| State | Memory |
| --- | --- |
| Between dictations | **nothing** on Linux; **~6 MB** on macOS / Windows (the hotkey listener) |
| While you talk | ~40 MB of its own (the pill and the recorder; the GPU libraries it shares with other apps not counted) |
| Transcribing (~0.1–0.8 s) | ~100 MB RAM + 450 MB GPU memory with Vulkan / Metal, ~330 MB RAM on the CPU |
| For 3 min after a paste (learning your fixes) | a few MB (a watcher with no UI) |

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

The cleanup model is the bigger cost when it runs locally: `qwen3.5:4b` in
Ollama takes 3.3 GB of disk and ~3.8 GB of GPU memory (or RAM) while loaded;
Ollama unloads it after `LOCALFLOW_KEEP_ALIVE` (1 hour, 2 minutes on macOS).
With your API key, none of that runs on your machine.

| Cleanup | Short sentence | Long ramble (~1 min of speech) |
| --- | --- | --- |
| qwen3.5:4b, GPU (RX 9070 XT) | ~0.4 s | ~1.5 s |
| qwen3.5:4b, CPU only | ~0.9 s | ~20 s |
| qwen3.5:4b, Apple M4 | ~0.5 s | ~5–7 s |
| Claude Haiku (API) | ~1.0–1.3 s | ~2 s |

## Use your own API key (optional)

The cleanup can run on a small hosted model with your own key: **Claude
Haiku**, or any model behind an OpenAI-compatible API. No Ollama and no local
cleanup model. The trade-off: **every dictation is sent to that provider**,
and it costs a fraction of a cent (Haiku 5.5: $0.10 / $0.50 per million input
/ output tokens; each dictation sends ~3,900 tokens of rules and examples,
cached between dictations, so under $0.0005 each).

Pick "With your API key" in the settings window, or:

```sh
LOCALFLOW_API_KEY=... localflow setup --api anthropic      # always claude-haiku-5-5
LOCALFLOW_API_KEY=... localflow setup --api openai --model <model>
LOCALFLOW_API_KEY=... localflow setup --api https://my-server.example/v1 --model <model>
```

`--api` takes `anthropic`, `openai`, `openrouter`, `groq`, `gemini`,
`mistral`, `deepseek`, or a base URL. With Anthropic the model is always
Claude Haiku 5.5, through Anthropic's own Messages API with thinking off and
the rules cached; everything else goes through the OpenAI-compatible chat API.
The key is saved to `~/.config/localflow/api_key` (`%APPDATA%\localflow\api_key`
on Windows), readable only by you. For other providers, pick their smallest
fast **non-reasoning** model.

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
over (and fix: localflow learns from that). "Send prompts to coding agents"
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

When localflow gets a word wrong and you fix it, it learns the fix. For 3
minutes after each paste (or until your next dictation), it looks for the
version you kept:

| Where you dictated | How it sees your fix | Linux | macOS | Windows |
| --- | --- | --- | --- | --- |
| Claude Code (terminal or editor panel), Codex | the message you sent, from its transcript | ✅ | ✅ | ✅ |
| Cursor's chat | the message you sent, from Cursor's chat history | ✅ | ✅ | ✅ |
| Any other text box | the box itself, once you send it or move on | – | ✅ | ✅ |

The words you changed go to the cleanup model, which keeps fixes of misheard
words ("Claw" to "Claude", "omar key" to "Omarchy") and drops changes of mind
("Tuesday" to "Wednesday"). Each fix is saved to your vocabulary,
`~/.config/localflow/words.txt` (`%APPDATA%\localflow\words.txt` on Windows),
as `Claude: often heard as "Claw"`, and from then on it's spelled right. To
teach it right away, or anywhere else, select the fixed sentence (or copy it)
and press the learn key: `Option+Shift+L` (macOS), `Ctrl+Alt+Space`
(Windows), or a key bound to `localflow learn` (Linux).

"Learn from your fixes" in the settings (`LOCALFLOW_LEARN`): `auto` (on its
own and with the key), `key` (only the key), `off`. What it reads stays on
your machine except the changed sentence, which goes to your cleanup model
once, for the check. Nothing is logged.

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
same without the window (`--api`, `--model`, `--speech-model`), downloads the
speech model and puts the app in place.

| Variable | What |
| --- | --- |
| `LOCALFLOW_SPEECH_MODEL` | `tiny.en`, `base.en`, `small.en` (default), `large-v3-turbo`, `base`, `small` (see `localflow models`) |
| `LOCALFLOW_SPEECH_GPU` | `off` to transcribe on the CPU |
| `LOCALFLOW_LANGUAGE` | what you speak (`en`, `de`, …; default: detected) |
| `LOCALFLOW_API_URL` | a provider or base URL (default: Ollama) |
| `LOCALFLOW_MODEL` | the cleanup model (default `qwen3.5:4b`; always Claude Haiku on Anthropic) |
| `LOCALFLOW_REVIEW_MODEL` | the repair pass's model, or `off` |
| `LOCALFLOW_ASIDES` | `keep` to paste asides as said |
| `LOCALFLOW_LEARN` | `auto`, `key` or `off` |
| `LOCALFLOW_SUBMIT` | in a coding agent, press Enter after pasting: `said` (when you end with "send it", default), `always`, `never` |
| `LOCALFLOW_HISTORY` | `off` to keep no history of your dictations |
| `LOCALFLOW_OLLAMA`, `LOCALFLOW_KEEP_ALIVE`, `LOCALFLOW_USE_MMAP` | Ollama's address, how long it keeps the model loaded, memory mapping |

Logs (never your words; those are in the [history](#history)) go to `~/.local/state/localflow/dictate.log`,
`~/Library/Logs/localflow/dictate.log` or `%LOCALAPPDATA%\localflow\dictate.log`.

## Tools and tuning

```sh
localflow models [download ID]       # speech models
localflow history [N]                # what you said and what went out
localflow listen 5                   # record 5 s, transcribe, clean up, print (segments and levels on stderr)
localflow transcribe talk.wav        # the same from a WAV file
echo "call mom tomorrow actually tonight" | localflow -
localflow --check                    # is the cleanup model reachable?
localflow --eval [MODEL [REVIEW|off]]   # cases.txt against the model
localflow --selftest                 # each piece that doesn't need a person
```

Rules and examples live in `crates/core/src/prompts.rs`. `cases.txt` holds
`raw => expected` cases, compared ignoring case and punctuation (list cases
compare the numbered items; `? +must; -must not; order: a < b` checks
properties of long dictation). After changing prompts, run `localflow --eval`
and `UPDATE_PARITY=1 cargo test -p localflow-core --test parity`.

## Testing

`cargo test --workspace` needs no key, model, microphone or network. CI
(`.github/workflows/ci.yml`) runs it, clippy, and `cargo xtask e2e` on fresh
Linux, macOS and Windows machines: it speaks a sentence with the OS's
text-to-speech and dictates it, through the hotkey (Linux: `start`/`stop`),
the pill, our speech engine, the cleanup (a fake API) and the paste, into a
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
  takes ~1 s there).

## Layout

| Path | What |
| --- | --- |
| `crates/core` | the cleanup: rules and examples, Ollama / Anthropic / OpenAI-compatible, review pass, learning |
| `crates/speech` | the microphone, the speech engine (whisper.cpp), its models, quiet segments |
| `crates/desktop` | where you're dictating, paste and clipboard, hotkeys, learning from your fixes |
| `crates/app` | `localflow`: the GPUI pill and settings window, sessions, setup, the CLI |
| `crates/xtask` | `cargo xtask package / e2e / installed` |
| `crates/testbox` | the text box the end-to-end test dictates into |
| `pkg/localflow.wxs` | the Windows installer |
| `words.txt`, `cases.txt` | the shipped vocabulary, the eval cases |
