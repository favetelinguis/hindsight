---
name: verify
description: Build, seed, and drive hindsight's zsh+fzf surface end-to-end in tmux to verify changes at the real TUI.
---

# Verifying hindsight changes

The surface is the zsh integration (Ctrl-R fzf picker, Up/Down search, Ctrl-O
context drill), not the Rust functions. Drive it in an isolated tmux session.

## Build + seed

```bash
cargo build                  # debug binary at target/debug/hindsight
./scripts/seed-dev-db.sh     # wipes and reseeds .dev/data/history.db
```

Everything below must run with `export _HINDSIGHT_DATA_DIR="$PWD/.dev/data"`
so the real DB is never touched. To seed extra commands (e.g. multiline):

```bash
./target/debug/hindsight start --session seed-1 --pwd "$PWD" -- $'line1\nline2'
./target/debug/hindsight end --session seed-1 --exit 0
```

## Drive the TUI

`scripts/dev-shell.sh` opens a sandboxed interactive zsh (scratch ZDOTDIR,
dev DB, debug binary on PATH). Run it inside tmux with a private socket:

```bash
tmux -L hsverify new-session -d -x 100 -y 30 "./scripts/dev-shell.sh"
tmux -L hsverify send-keys C-r          # open picker (C-s star, C-e note, C-t preview, C-o drill)
tmux -L hsverify capture-pane -p        # evidence
tmux -L hsverify kill-server            # cleanup
```

Sleep ~1.5s after each keypress before capturing. `Escape` closes fzf.
Check side effects directly: `sqlite3 .dev/data/history.db "SELECT quote(cmd) FROM favorites;"`.

## Gotchas

- The picker feeds fzf NUL-terminated `<marker>\t<cmd>` records (`--read0`);
  inspect raw framing with `hindsight picker --state /tmp/x --session s | od -c`.
- Ctrl-E needs `$EDITOR` set *inside* the dev shell (send an `export EDITOR=...`
  line first) — but that export becomes the newest history entry, so the picker
  selection lands on it; press `Down` to reach the entry you actually want.
- Accepted commands land in the zsh buffer; press Enter to run them, which also
  exercises the preexec/precmd capture path into the dev DB.
