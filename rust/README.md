# localflow in Rust

The rewrite of localflow in Rust: a native app on Linux, macOS and Windows,
with no Python inside, a small binary and low memory use. The Python version
(the repo root, and the Windows `.msi` built from it) is what ships until this
one does everything it does.

```sh
cargo test                                  # no key, model, microphone or network needed
cargo run -p localflow -- listen 5          # record 5 s, transcribe with Handy, clean up
cargo run -p localflow -- transcribe a.wav  # the same from a 16-bit WAV file
cargo run -p localflow -- --check           # is the cleanup model reachable?
echo "meet at 3 wait at 4" | cargo run -p localflow
cargo run -p localflow --release -- --eval  # cases.txt against your model
```

On Linux, building needs ALSA's headers (`alsa-lib` on Arch, `libasound2-dev`
on Debian/Ubuntu); PipeWire serves them through its ALSA plugin.

Settings are the same as the Python version's (`LOCALFLOW_*` variables, or the
env file `install.py` writes), so both run against the same setup.

## Status

| Piece | Python | Rust |
| --- | --- | --- |
| Cleanup: rules, few-shot examples, Ollama / Anthropic / OpenAI-compatible, review pass, sanity checks | `localflow` | done: `crates/core` |
| Learning: corrections, mishearings, words.txt | `localflow` | done: `crates/core` |
| `--check`, `--eval`, stdin filter | `localflow` | done: `crates/app` |
| Recording (PipeWire/ALSA, WASAPI, Core Audio), level meter | `dictate` | done: `crates/speech` |
| Transcription (Handy's engine) | `dictate` | done: `crates/speech` |
| Overlay pill | `dictate` | next |
| Hotkeys (Windows, macOS; compositor keys on Linux) | `dictate` | to do |
| Paste, clipboard, file attachments | `dictate`, `paste` | to do |
| Where you're dictating: app, project, files | `dictate` | to do |
| Learning on its own (UI Automation, editor and Claude Code logs) | `dictate` | to do |
| Setup window, installers (.msi, .dmg, AppImage) | `pkg/`, `install.py` | to do |
| Handy proxy (`serve`, `--warm`) | `localflow` | only if `--handy-app` stays |

## Staying the same as Python

The cleanup must behave exactly like the Python version, so the prompts in
`crates/core/src/prompts.rs` are generated from it, and
`crates/core/tests/parity.rs` checks the Rust functions against what Python
returns for about a thousand cases (`parity.json`). After changing the Python
`localflow`, regenerate the fixture and port the change until it passes:

```sh
python3 crates/core/tests/parity.py && cargo test
```

CI (`.github/workflows/rust.yml`) fails if the fixture is out of date.

## Layout

| Path | What |
| --- | --- |
| `crates/core` | `localflow-core`: the cleanup engine and learning, as a library |
| `crates/speech` | `localflow-speech`: the microphone as 16 kHz mono, its level, Handy transcription |
| `crates/app` | `localflow`: the binary (the CLI now; the dictation app next) |
