# hindsight

A fast command-history recorder and search tool for zsh.

`hindsight` records **every command you run** — along with the directory it ran in, its exit code, and
timestamps — into a local SQLite database. It then lets you fuzzy-search that history, keep
**favorites**, attach **notes** to commands, and expose rich usage **metadata as JSON** for AI
agents.

It rebinds **Ctrl-r** to an [fzf](https://github.com/junegunn/fzf) picker and **Up/Down** to a
cwd-aware prefix search.

---

## Why hindsight exists

Plenty of tools help you run a set of **predefined**, curated commands — task runners like `just`,
`make`, Makefile-style recipes, shell aliases. They're great when you already know, up front, which
workflows are worth capturing.

But in a real enterprise setting, that's rarely how the important stuff surfaces. The most valuable
commands are usually the ones you **didn't know were important at the time**: the magic
three-command incantation a colleague pasted into your terminal, the exact `kubectl`/`aws`/`docker`
sequence that unstuck a deploy, the one-off build flags that finally worked. You don't think to
write those down — until weeks or months later, when you need to run them again or explain what they
actually did, and they're gone.

`hindsight` is built for that **after-the-fact** case. Instead of asking you to decide in advance what
matters, it quietly records **everything** you run — the command, the directory, the exit code, when
it ran, and the session it belonged to — so the decision about what's important can be made *later*,
whenever the need arises. The goal is simple: **never lose a command you've run**, and be able to go
back and understand earlier commands no matter how long ago you used them.

That shapes every feature here:

- **Total capture by default** — nothing is curated up front; it's all captured, then searchable.
- **Reconstruct the workflow, not just the command** — [usage context](#usage-context-ctrl-o) lets
  you see a command *in situ*: what was run before and after it, in which project and terminal, so
  you can rebuild that "magic workflow" a colleague showed you months ago.
- **Annotate once you understand it** — [notes](#notes) let you record *why* a command mattered the
  moment you figure it out, so future-you (or a teammate) isn't reverse-engineering it again.
- **Nothing is ever truly destroyed** — [soft delete](#soft-delete-nothing-is-ever-destroyed) hides
  noise from your views without discarding data, because you can't know today what you'll wish you
  still had tomorrow.
- **Queryable by tools and agents** — rich [JSON metadata](#for-ai-agents) so an AI assistant can
  reconstruct and explain your past workflows for you.

---

## How it works

`hindsight` follows the same shape as tools like zoxide: a single self-contained binary that prints
shell integration code, which you `eval` in your `~/.zshrc`.

- **Recording.** The integration installs two zsh hooks:
  - `preexec` runs `hindsight start` just before a command executes, saving the command text, `$PWD`,
    and a start time as a _pending_ entry (keyed by a per-shell session id).
  - `precmd` runs `hindsight end` right after it finishes, recording the exit code (`$?`) and
    finalizing the entry.
- **Storage.** Everything lives in one SQLite database. Location (resolved in this order):
  1. `$_HINDSIGHT_DATA_DIR` if set, otherwise
  2. the platform data dir:
     - macOS: `~/Library/Application Support/hindsight/history.db`
     - Linux: `$XDG_DATA_HOME/hindsight/history.db` or `~/.local/share/hindsight/history.db`
- **Retrieval.**
  - **Ctrl-r** opens an fzf picker over your history.
  - **Up / Down arrows** do a cwd-aware prefix search: type part of a command, press Up to cycle
    older matches (commands run in the current directory rank first), Down to come back.

All state is local; nothing leaves your machine.

---

## Requirements

- **zsh**
- **[fzf](https://github.com/junegunn/fzf)** — required. The Ctrl-r picker pipes history into `fzf`.
  Install it first, e.g. `brew install fzf`.
- An **`$EDITOR`** set if you want to edit notes (see [Notes](#notes)). `hindsight` uses `$EDITOR`
  only — it does **not** fall back to `$VISUAL` or `vi`.

---

## Installation

### With mise (prebuilt binary, recommended)

Tagged releases publish prebuilt binaries to GitHub Releases (macOS arm64/x86_64, Linux
arm64/x86_64). [mise](https://mise.jdx.dev) installs the right one for your platform via its
`github` backend — no Rust toolchain needed:

```sh
mise use -g "github:favetelinguis/hindsight@latest"   # or pin e.g. @0.1.0
```

Or add it to `~/.config/mise/config.toml` (or a project `mise.toml`):

```toml
[tools]
"github:favetelinguis/hindsight" = "latest"
```

`mise` puts `hindsight` on your `PATH`. (Release assets are named `hindsight-<target-triple>.tar.gz`, so
mise auto-detects your OS/arch; each ships with a `.sha256` checksum.)

> **Note:** mise applies a `minimum_release_age` cooldown to freshly published releases as a
> supply-chain precaution. If a just-cut release doesn't resolve ("no versions found matching date
> filter"), either wait it out or exclude this tool in your mise settings:
>
> ```toml
> [settings]
> minimum_release_age_excludes = ["github:favetelinguis/hindsight"]
> ```

### From source with Cargo

```sh
git clone https://github.com/favetelinguis/hindsight ~/code/hindsight
cd ~/code/hindsight
cargo install --path .
```

This puts `hindsight` on your `PATH` (typically `~/.cargo/bin/hindsight`). You can also build straight from
the repo with mise's cargo backend: `mise use -g "cargo:https://github.com/favetelinguis/hindsight"`.

### Then: shell integration

Add this to the **end** of your `~/.zshrc`:

```sh
eval "$(hindsight init zsh)"
```

Open a new terminal (or `source ~/.zshrc`) and you're recording.

### Ordering matters: source hindsight **after** fzf

Both fzf and hindsight bind **Ctrl-r**, and in zsh **the last binding wins**. fzf's own integration
(`eval "$(fzf --zsh)"`) binds `^R` to its history widget; hindsight binds `^R` to its picker. To make
**hindsight** own Ctrl-r, its `eval` line must come **after** fzf's in your `~/.zshrc`:

```sh
# fzf first…
eval "$(fzf --zsh)"

# …hindsight last, so its Ctrl-r binding overrides fzf's
eval "$(hindsight init zsh)"
```

> Verified: `fzf --zsh` runs `bindkey -M emacs/viins/vicmd '^R' fzf-history-widget`; hindsight runs
> `bindkey '^R' __hindsight_search_widget`. Whichever `eval` executes later is the binding that stays
> in effect. If Ctrl-r still opens fzf's plain history, hindsight was sourced too early.

hindsight still _uses_ fzf internally for its picker — it just replaces the **keybinding**, not fzf
itself.

---

## Usage (interactive)

### Ctrl-r — the picker

Press **Ctrl-r** to open the fzf picker. Rows are prefixed with markers: `★` = favorite, `✎` = has
a note. In-picker keys:

| Key      | Action                                                                |
| -------- | --------------------------------------------------------------------- |
| `Ctrl-r` | Toggle between **history** view and **favorites-only** view           |
| `Ctrl-s` | Star / unstar the highlighted command (create/remove a favorite)      |
| `Ctrl-e` | Edit the highlighted command's note in `$EDITOR`                      |
| `Ctrl-t` | Show / hide the note preview pane (bottom, hidden by default)         |
| `Ctrl-k` | Soft-delete the highlighted command (hidden from views; data kept)    |
| `Ctrl-o` | Explore how the command was used across sessions (usage context)      |
| `Enter`  | Put the selected command on your prompt                               |

`Ctrl-k` is a **soft delete** — see [Soft delete](#soft-delete-nothing-is-ever-destroyed).

### Up / Down arrows

Type a prefix and press **Up** to walk older matching commands (current-directory matches first,
then global); **Down** walks back toward newer, and returns to exactly what you typed at the top.

### Favorites

Star from the picker with `Ctrl-s`, or from the CLI:

```sh
hindsight fav add    -- git push
hindsight fav rm     -- git push
hindsight fav toggle -- git push
hindsight fav list
```

### Notes

Attach a note to **any** command (favorite or not). Edit with `Ctrl-e` in the picker, or via CLI:

```sh
hindsight note show  -- "git push"                    # print the note (or a placeholder)
hindsight note edit  -- "git push"                    # open the note in $EDITOR
hindsight note set   --note "force-push after rebase" -- "git push"
hindsight note clear -- "git push"
```

`hindsight note edit` (and `Ctrl-e`) require `$EDITOR` to be set; if it isn't, hindsight prints a short
error and does nothing.

### Usage context (Ctrl-o)

A command is often run in many different terminal sessions, surrounded by different commands. Press
**Ctrl-o** on a highlighted command to explore that:

- A second fzf opens listing every **session** the command ran in, newest first, each row showing
  when it last ran there (`2026-07-17 21:40`, local time, 24h clock), its starting directory (e.g.
  `~/repo`), and the occurrence count.
- The **preview pane** shows that session's **full timeline** — every command in the session in
  order, each with its directory and exit code, and the matched command marked `→`.
- **Up / Down** switches between sessions; the preview updates live.
- **Ctrl-e** opens the highlighted session's whole timeline in `$EDITOR`, so you can read it and
  copy-paste commands out. (Read-only — hindsight doesn't save anything back.)
- **Esc** returns to the main picker.

A session is one shell process: its id is derived from the shell's start time and PID, independent
of any terminal emulator or multiplexer. Sessions are labeled by the first directory recorded in
them; sessions with no recorded starting directory (e.g. imported history) fall back to the raw
session id. Soft-deleted commands are excluded from timelines.

### Other commands

```sh
hindsight query --list                 # newest-first, deduped command list
hindsight delete  -- "docker ps"       # soft-delete: hide from views (data kept)
hindsight restore -- "docker ps"       # un-hide a soft-deleted command
hindsight import                       # seed from ~/.zsh_history (best-effort; safe to re-run)
```

Run `hindsight --help` or `hindsight <command> --help` for full details.

---

## Ignoring commands

Some commands add noise and aren't worth recording (e.g. zoxide's `z` / `zi`). List regexes to
ignore in `hindsight.toml`:

- `$XDG_CONFIG_HOME/hindsight/hindsight.toml` if `XDG_CONFIG_HOME` is set, otherwise
- `~/.config/hindsight/hindsight.toml`

```toml
# Commands matching any of these regexes are not recorded, and are soft-deleted
# by `hindsight prune ignore`. Patterns are UNANCHORED — anchor with ^ / $ yourself.
ignore = [
  '^z( |$)',      # the `z` command (and `z foo`) — but not `zip`/`fzf`
  '^zi( |$)',     # zoxide interactive
  '^ls\b',        # ls, ls -la, …
  'secret',       # any command containing "secret"
]
```

Matching is an **unanchored regex search** (a pattern matches if found anywhere in the command), so
you control anchoring:

| Pattern    | Matches                              | Does **not** match |
| ---------- | ------------------------------------ | ------------------ |
| `^z( \|$)`  | `z`, `z proj`                        | `zip`, `fzf`       |
| `^zi( \|$)` | `zi`, `zi bar`                       | `zip`              |
| `^ls\b`    | `ls`, `ls -la`                       | `lsof`             |
| `secret`   | `aws secret get`, `echo $SECRET`     |                    |

Ignored commands are:

- **not recorded going forward** — filtering happens in `hindsight start` (the preexec hook), so they
  never enter the database; and
- **removed from existing history** by `hindsight prune ignore` (below).

A missing config file, or a missing `ignore` key, simply means nothing is ignored. An invalid regex
is skipped with a warning during recording (recording is never blocked), and is a hard error for
`hindsight prune ignore`.

### `hindsight prune ignore`

Soft-deletes commands already in the database that match the ignore list. **Dry run by default** —
it prints what *would* be removed and changes nothing:

```sh
hindsight prune ignore            # dry run: shows what would be soft-deleted
hindsight prune ignore --apply    # actually soft-delete the matches
```

---

## Soft delete: nothing is ever destroyed

hindsight **never physically deletes command data**. `Ctrl-k`, `hindsight delete`, and
`hindsight prune ignore --apply` all **soft-delete**: the command is marked hidden and disappears from
every user-facing surface (the picker, `query --list`, Up/Down search, favorites, and the
`inspect` / `stats` agent commands). The underlying history, favorite, and note rows stay in the
database.

- **Restore** anything: `hindsight restore -- "<command>"` brings it back into normal views.
- **Inspect** hidden commands (for AI agents): `hindsight deleted list` and
  `hindsight deleted inspect -- "<command>"` (see below).

---

## For AI agents

`hindsight` exposes read-only, **JSON-only** commands so an agent (e.g. Claude Code) can analyze your
shell usage. `hindsight --help` has a dedicated "FOR AI AGENTS" section, and every subcommand's
`--help` documents its exact output. Pass a command **verbatim after `--`**.

### `hindsight inspect -- <command>`

All metadata for **one** command, as a single JSON object:

```jsonc
{
  "command": "git push",
  "found": true, // false ⇒ unknown command, no other fields
  "run_count": 42,
  "exit_codes": { "0": 38, "1": 4 }, // code → count; key "null" = never finished
  "directories": [
    // most-used first
    { "cwd": "/repo", "count": 40 },
    { "cwd": "/tmp", "count": 2 },
  ],
  "note": "force-push after rebase", // or null
  "is_favorite": true,
  "first_run": 1710000000, // unix seconds (or null)
  "last_run": 1710500000,
}
```

### `hindsight stats [--limit N]`

Global aggregates across all history, as a single JSON object (default `--limit 20`):

```jsonc
{
  "totals": {
    "distinct_commands": 512,
    "total_runs": 9001,
    "favorites": 7,
    "notes": 3,
  },
  "top_commands": [{ "command": "ls", "count": 300 }],
  "error_prone": [{ "command": "cargo build", "failures": 12, "runs": 40 }],
  "top_directories": [{ "cwd": "/repo", "count": 1200 }],
}
```

`inspect` and `stats` reflect **active** history only — soft-deleted commands are excluded.

### `hindsight deleted list` / `hindsight deleted inspect -- <command>`

Soft-deleted commands are hidden from users but remain in the database. These agent-only commands
reach them:

```jsonc
// hindsight deleted list  →  array of soft-deleted commands
[{ "command": "z old", "deleted_ts": 1710500000, "run_count": 12 }]

// hindsight deleted inspect -- "z old"  →  same shape as `inspect`, plus:
{ "command": "z old", "found": true, "run_count": 12, /* … */
  "deleted": true, "deleted_ts": 1710500000 }
```

Use these to recover a command the user removed, or to understand history the user chose to hide.
`hindsight restore -- "<command>"` brings one back into normal views.

### `hindsight context json -- <command>`

How a command was used across sessions — the sessions it ran in and each session's whole timeline:

```jsonc
{
  "command": "cargo build",
  "found": true, // false ⇒ no active occurrences
  "sessions": [
    // newest first
    {
      "session": "…", // opaque session id
      "label": "~/repo", // the session's starting directory
      "count": 3, // times the command ran in this session
      "last_run": 1710000005, // unix timestamp (seconds) of the most recent run in this session
      "timeline": [
        // the whole session, in order
        { "cmd": "git pull", "cwd": "~/repo", "exit_code": 0, "start_ts": 1710000000, "is_match": false },
        { "cmd": "cargo build", "cwd": "~/repo", "exit_code": 0, "start_ts": 1710000005, "is_match": true }
      ]
    }
  ]
}
```

Use this to understand a command's typical surrounding workflow — what's run just before/after it,
in which projects and terminals, and whether it tends to succeed there.

### Intended agent workflow

- **Understand a command before running/suggesting it** — `hindsight inspect -- <cmd>` to see how often
  it's used, whether it usually succeeds (`exit_codes`), where it runs (`directories`), and any note
  the user attached.
- **Learn the user's environment** — `hindsight stats` for the most-used commands, the most
  error-prone ones (good candidates to double-check or fix), and the busiest directories.
- **Read user intent from notes** — the `note` field often records _why_ a command exists or a
  caveat ("use with care on shared branches").
- **Understand a command's workflow** — `hindsight context json -- <cmd>` shows the sessions it ran in
  and the commands around it, so you can infer the surrounding routine (setup, follow-ups, project).
- **Find hidden history** — `hindsight deleted list` / `deleted inspect` surface commands the user
  soft-deleted, which `inspect`/`stats` deliberately omit.
- All output is JSON on stdout with a stable schema; parse it directly. Unknown commands return
  `{"command": "...", "found": false}` with exit code 0 (a clean answer, not an error).

---

## Uninstall

```sh
cargo uninstall hindsight
```

Remove the `eval "$(hindsight init zsh)"` line from `~/.zshrc`. To also drop your recorded data, delete
the data directory (see [How it works](#how-it-works)).
