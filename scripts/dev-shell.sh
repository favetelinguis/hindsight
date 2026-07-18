#!/usr/bin/env bash
# Open a sandboxed interactive shell wired to the debug build and the dev DB.
#
# Usage: dev-shell.sh [zsh|bash]   (default: zsh)
#
# The sandbox uses a scratch ZDOTDIR (zsh) or --rcfile (bash), so your real
# ~/.zshrc / ~/.bashrc is never sourced and the real hindsight install (binary,
# keybindings, database) is untouched. Inside it, every command is recorded to
# .dev/data/history.db and Ctrl-R / Ctrl-O / Up-arrow all run against
# target/debug/hindsight. Leave with `exit` or Ctrl-D.
#
# fzf must be on your PATH (it is inherited from this shell).
set -euo pipefail

shell="${1:-zsh}"
repo_root="$(cd "$(dirname "$0")/.." && pwd)"

if [[ ! -x "$repo_root/target/debug/hindsight" ]]; then
    echo "error: target/debug/hindsight not found — run 'mise run build' first" >&2
    exit 1
fi

case "$shell" in
zsh)
    zdotdir="$repo_root/.dev/zdotdir"
    mkdir -p "$zdotdir"
    cat > "$zdotdir/.zshrc" <<EOF
export PATH="$repo_root/target/debug:\$PATH"
export _HINDSIGHT_DATA_DIR="$repo_root/.dev/data"
PROMPT='%F{yellow}hindsight-dev%f %~ %# '
eval "\$(hindsight init zsh)"
EOF
    echo "Entering hindsight dev shell (zsh, dev DB: .dev/data). Type 'exit' to leave."
    exec env ZDOTDIR="$zdotdir" zsh -i
    ;;
bash)
    bash_major="$(bash -c 'echo "${BASH_VERSINFO[0]}"')"
    if ((bash_major < 5)); then
        echo "error: the hindsight bash integration needs bash >= 5.0 (found $(bash -c 'echo "$BASH_VERSION"'))" >&2
        exit 1
    fi
    mkdir -p "$repo_root/.dev"
    rcfile="$repo_root/.dev/bashrc"
    # History stays ON (recording reads the typed line from history) but goes
    # to a scratch file so the real ~/.bash_history is untouched.
    cat > "$rcfile" <<EOF
export PATH="$repo_root/target/debug:\$PATH"
export _HINDSIGHT_DATA_DIR="$repo_root/.dev/data"
HISTFILE="$repo_root/.dev/bash_history"
PS1='\[\e[33m\]hindsight-dev\[\e[0m\] \w \\\$ '
eval "\$(hindsight init bash)"
EOF
    echo "Entering hindsight dev shell (bash, dev DB: .dev/data). Type 'exit' to leave."
    exec bash --noprofile --rcfile "$rcfile" -i
    ;;
*)
    echo "usage: $0 [zsh|bash]" >&2
    exit 2
    ;;
esac
