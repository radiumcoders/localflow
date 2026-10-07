# handy-clean

Local dictation cleanup for [Handy](https://handy.computer) on Omarchy.

Handy transcribes speech (Parakeet V3), then sends the text to a small local
LLM in Ollama that keeps only what you meant:

    "Let's meet in three hours. Wait, in two hours."  →  "Let's meet in two hours."
    "Um, ship it on, uh, Monday."                      →  "Ship it on Monday."

Everything runs on this machine; nothing leaves it.

## Keys (Hyprland, `~/.config/hypr/bindings.lua`)

| Key              | Action                         |
| ---------------- | ------------------------------ |
| `SUPER+CTRL+X`   | toggle dictation               |
| `F9` (hold)      | push-to-talk                   |

Both call `handy --toggle-post-process`, which records, transcribes and then
runs the cleanup model. Handy starts hidden at login from
`~/.config/hypr/autostart.lua`.

## Pieces

| File             | What                                                              |
| ---------------- | ----------------------------------------------------------------- |
| `Modelfile`      | Ollama model `handy-clean`: base model + rules, examples, temp 0   |
| `handy-clean`    | sends text to the model exactly as Handy does; `--eval` runs tests |
| `cases.txt`      | eval cases (`raw => expected`)                                     |
| `ollama.service` | user-level Ollama (ROCm build in `~/.local/ollama`)                |
| `install.sh`     | sets everything up; safe to re-run                                 |

Handy is configured with the **Custom** post-processing provider
(`http://localhost:11434/v1`), model `handy-clean`, and the prompt
`<dictation>${output}</dictation>`. The rules live in the Modelfile because
Handy can't set temperature or few-shot examples itself.

## Install

```sh
# Ollama (no sudo): official release + ROCm add-on for the RX 9070
mkdir -p ~/.local/ollama
for f in ollama-linux-amd64 ollama-linux-amd64-rocm; do
  curl -fsSL https://github.com/ollama/ollama/releases/latest/download/$f.tar.zst | zstd -d | tar -x -C ~/.local/ollama
done

# Handy AppImage
mkdir -p ~/.local/share/handy
curl -fL -o ~/.local/share/handy/Handy.AppImage \
  https://github.com/cjpais/Handy/releases/latest/download/Handy_0.9.8_amd64.AppImage
chmod +x ~/.local/share/handy/Handy.AppImage

./install.sh
```

## Tuning

```sh
echo "call mom tomorrow actually tonight" | ./handy-clean
./handy-clean --eval                        # current handy-clean model
./handy-clean --eval qwen3.5:4b             # compare a raw base model
BASE_MODEL=qwen3.5:4b ./install.sh          # rebuild handy-clean on a bigger base
```

Edit the rules/examples in `Modelfile`, re-run `./install.sh`, then `--eval`.
