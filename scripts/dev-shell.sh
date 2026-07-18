#!/usr/bin/env bash
# Open a sandboxed interactive zsh wired to the debug build and the dev DB.
#
# The sandbox uses a scratch ZDOTDIR, so your real ~/.zshrc is never sourced
# and the real hindsight install (binary, keybindings, database) is untouched.
# Inside it, every command is recorded to .dev/data/history.db and Ctrl-R /
# Ctrl-O / Up-arrow all run against target/debug/hindsight. Leave with `exit`
# or Ctrl-D.
#
# fzf must be on your PATH (it is inherited from this shell).
set -euo pipefail

repo_root="$(cd "$(dirname "$0")/.." && pwd)"

if [[ ! -x "$repo_root/target/debug/hindsight" ]]; then
    echo "error: target/debug/hindsight not found — run 'mise run build' first" >&2
    exit 1
fi

zdotdir="$repo_root/.dev/zdotdir"
mkdir -p "$zdotdir"
cat > "$zdotdir/.zshrc" <<EOF
export PATH="$repo_root/target/debug:\$PATH"
export _HINDSIGHT_DATA_DIR="$repo_root/.dev/data"
PROMPT='%F{yellow}hindsight-dev%f %~ %# '
eval "\$(hindsight init zsh)"
EOF

echo "Entering hindsight dev shell (dev DB: .dev/data). Type 'exit' to leave."
exec env ZDOTDIR="$zdotdir" zsh -i
