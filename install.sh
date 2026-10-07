#!/usr/bin/env bash
# Set up Handy + local Ollama dictation cleanup. Safe to re-run.
#
#   ./install.sh                      # base model qwen3.5:4b
#   BASE_MODEL=qwen3.5:2b ./install.sh   # smaller, faster, less accurate
#
# Expects Ollama extracted to ~/.local/ollama and the Handy AppImage at
# ~/.local/share/handy/Handy.AppImage (see README.md).
# Hyprland keybinds/autostart live in ~/.config/hypr (see README.md).
set -euo pipefail

here="$(cd "$(dirname "$0")" && pwd)"
base_model="${BASE_MODEL:-qwen3.5:4b}"
ollama="$HOME/.local/ollama/bin/ollama"
appimage="$HOME/.local/share/handy/Handy.AppImage"
appdir="$HOME/.local/share/handy/app"
store="$HOME/.local/share/com.pais.handy/settings_store.json"
prompt='<dictation>${output}</dictation>'  # handy-clean serve unwraps this

[[ -x $ollama ]] || { echo "ollama not found at $ollama" >&2; exit 1; }
[[ -x $appimage ]] || { echo "Handy not found at $appimage" >&2; exit 1; }
mkdir -p ~/.local/bin ~/.config/systemd/user

# --- Ollama: user service (skipped if some ollama already serves :11434) ---
if ! curl -sf -m 2 127.0.0.1:11434/api/version >/dev/null; then
  ln -sf "$here/ollama.service" ~/.config/systemd/user/ollama.service
  systemctl --user daemon-reload
  systemctl --user enable --now ollama.service
  for _ in $(seq 30); do curl -sf -m 1 127.0.0.1:11434/api/version >/dev/null && break; sleep 0.5; done
fi
ln -sf "$ollama" ~/.local/bin/ollama

# --- Cleanup proxy: Handy -> handy-clean (:11435) -> Ollama native API ---
"$ollama" pull "$base_model"
sed -i "s|^Environment=HANDY_CLEAN_MODEL=.*|Environment=HANDY_CLEAN_MODEL=$base_model|" "$here/handy-clean.service"
ln -sf "$here/handy-clean.service" ~/.config/systemd/user/handy-clean.service
systemctl --user daemon-reload
systemctl --user enable handy-clean.service
systemctl --user restart handy-clean.service

# --- Handy: extract the AppImage so each keypress skips a FUSE mount ---
pkill -x handy 2>/dev/null && sleep 1 || true
rm -rf "$appdir" "$(dirname "$appimage")/squashfs-root"
(cd "$(dirname "$appimage")" && "$appimage" --appimage-extract >/dev/null && mv squashfs-root "$appdir")
printf '#!/bin/sh\nexec "%s/AppRun" "$@"\n' "$appdir" >~/.local/bin/handy
chmod +x ~/.local/bin/handy

mkdir -p ~/.local/share/applications
printf '%s\n' "[Desktop Entry]" "Type=Application" "Name=Handy" "Comment=Speech to text" \
  "Exec=$HOME/.local/bin/handy" "Icon=$appdir/handy.png" "Categories=Utility;" \
  >~/.local/share/applications/handy.desktop

# Handy writes its settings store on first launch; create it, then patch it.
if [[ ! -f $store ]]; then
  ~/.local/bin/handy --start-hidden >/dev/null 2>&1 &
  for _ in $(seq 40); do [[ -f $store ]] && break; sleep 0.5; done
  sleep 2
  pkill -x handy || true
  sleep 1
fi

PROMPT="$prompt" STORE="$store" python3 - <<'EOF'
import json, os
path, prompt = os.environ["STORE"], os.environ["PROMPT"]
data = json.load(open(path))
s = data["settings"]
s["post_process_enabled"] = True
s["post_process_provider_id"] = "custom"
s.setdefault("post_process_models", {})["custom"] = "handy-clean"
for p in s.get("post_process_providers", []):
    if p["id"] == "custom":
        p["base_url"] = "http://127.0.0.1:11435/v1"
prompts = [p for p in s.get("post_process_prompts", []) if p["id"] != "handy_clean"]
prompts.append({"id": "handy_clean", "name": "Clean dictation (local)", "prompt": prompt})
s["post_process_prompts"] = prompts
s["post_process_selected_prompt_id"] = "handy_clean"
# Paste instead of typing: typed newlines become Enter (sends chat messages,
# runs shell commands). Shift+Insert pastes in terminals and GUI apps alike,
# and Handy restores the previous clipboard afterwards.
s["paste_method"] = "shift_insert"
s["start_hidden"] = True  # started by Hyprland at login; lives in the tray
json.dump(data, open(path, "w"), indent=2)
print("patched", path)
EOF

setsid ~/.local/bin/handy --start-hidden >/dev/null 2>&1 &
echo "done. test the model: echo \"let's meet in 3 hours wait in 2 hours\" | $here/handy-clean"
