# handy-clean

Local dictation cleanup for [Handy](https://handy.computer) on Omarchy.

Handy transcribes speech (your selected model, e.g. Parakeet), then a small
local LLM keeps only what you meant and formats lists:

    "let's meet in 3 hours wait in 2 hours"        →  Let's meet in 2 hours.
    "um I think we should uh ship it on Monday"    →  I think we should ship it on Monday.
    "my tasks today are first buy milk second
     call the bank third fix the login bug"        →  My tasks today are:
                                                      1. Buy milk
                                                      2. Call the bank
                                                      3. Fix the login bug

Everything runs on this machine (RX 9070 XT via Vulkan); nothing leaves it.

## How it fits together

    Handy ──(OpenAI API)──▶ handy-clean serve :11435 ──(native API)──▶ Ollama :11434 (qwen3.5:4b)

Handy's **Custom** post-process provider points at `http://127.0.0.1:11435/v1`
with model `handy-clean` and prompt `<dictation>${output}</dictation>`.
The proxy exists because Ollama's OpenAI endpoint ignores Modelfile
temperature (same dictation → different output each time) and few-shot
history. The proxy calls Ollama's native API with temperature 0, the rules and
examples in `handy-clean`, and falls back to the raw transcript if the model
replies with something much longer than you said or Ollama is down.

Handy pastes with **Shift+Insert** (works in terminals and GUI apps, keeps
your clipboard). Typing via wtype would turn list newlines into Enter presses.

## Keys (Hyprland, `~/.config/hypr/bindings.lua`)

| Key            | Action             |
| -------------- | ------------------ |
| `SUPER+CTRL+X` | toggle dictation   |
| `F9` (hold)    | push-to-talk       |

Both run `handy --toggle-post-process`. Handy starts hidden at login from
`~/.config/hypr/autostart.lua`.

## Files

| File                  | What                                                     |
| --------------------- | -------------------------------------------------------- |
| `handy-clean`         | prompt + examples, `serve` proxy, stdin filter, `--eval` |
| `cases.txt`           | eval cases (`raw => expected`, `\n` for list lines)      |
| `handy-clean.service` | user service for the proxy                               |
| `ollama.service`      | user-level Ollama from `~/.local/ollama`                 |
| `install.sh`          | sets everything up; safe to re-run                       |

## Install

```sh
# Ollama (no sudo): official release (includes Vulkan)
mkdir -p ~/.local/ollama
curl -fsSL https://github.com/ollama/ollama/releases/latest/download/ollama-linux-amd64.tar.zst \
  | zstd -d | tar -x -C ~/.local/ollama

# Handy AppImage
mkdir -p ~/.local/share/handy
curl -fL -o ~/.local/share/handy/Handy.AppImage \
  https://github.com/cjpais/Handy/releases/download/v0.9.8/Handy_0.9.8_amd64.AppImage
chmod +x ~/.local/share/handy/Handy.AppImage

./install.sh                         # qwen3.5:4b
BASE_MODEL=qwen3.5:2b ./install.sh   # faster, a bit less accurate
```

## Tuning

```sh
echo "call mom tomorrow actually tonight" | ./handy-clean
./handy-clean --eval                 # qwen3.5:4b: 18/21, ~270ms each
./handy-clean --eval qwen3.5:2b      #             16/21, ~150ms each
journalctl --user -u handy-clean -f  # per-request latency / Ollama errors
```

Edit `SYSTEM` / `EXAMPLES` in `handy-clean`, run `--eval`, then
`systemctl --user restart handy-clean`.
