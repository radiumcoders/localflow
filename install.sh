#!/usr/bin/env bash
# Install voxclean: user-level Ollama service + model + voxtype post-process hook.
# Ollama itself must already be extracted to ~/.local/ollama (see README).
set -euo pipefail

here="$(cd "$(dirname "$0")" && pwd)"
model="${VOXCLEAN_MODEL:-qwen3.5:2b}"
ollama="$HOME/.local/ollama/bin/ollama"
voxconf="$HOME/.config/voxtype/config.toml"

[[ -x $ollama ]] || { echo "ollama not found at $ollama" >&2; exit 1; }

# Skip the user service if a system-wide ollama is already serving.
if ! curl -sf -m 2 127.0.0.1:11434/api/version >/dev/null; then
  mkdir -p ~/.config/systemd/user
  ln -sf "$here/ollama.service" ~/.config/systemd/user/ollama.service
  systemctl --user daemon-reload
  systemctl --user enable --now ollama.service
  for _ in $(seq 30); do curl -sf -m 1 127.0.0.1:11434/api/version >/dev/null && break; sleep 0.5; done
fi

"$ollama" pull "$model"

# Hook voxclean into voxtype, once.
if ! grep -q '^\[output.post_process\]' "$voxconf"; then
  cp "$voxconf" "$voxconf.pre-voxclean"
  cat >>"$voxconf" <<EOF

[output.post_process]
command = "VOXCLEAN_MODEL=$model $here/voxclean"
timeout_ms = 10000
EOF
  echo "added [output.post_process] to $voxconf (backup: $voxconf.pre-voxclean)"
fi
systemctl --user restart voxtype.service

echo "done. test: echo \"let's meet in 3 hours wait in 2 hours\" | $here/voxclean"
