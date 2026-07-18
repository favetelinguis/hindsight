#!/usr/bin/env bash
# Seed the local dev database (.dev/data/history.db) with realistic history:
# several sessions across different directories and days, mixed exit codes,
# a favorite, and a note. Never touches the real database — everything goes
# through _HINDSIGHT_DATA_DIR.
#
# Re-running wipes the dev DB first, so the result is always a known state.
set -euo pipefail

repo_root="$(cd "$(dirname "$0")/.." && pwd)"
hs="$repo_root/target/debug/hindsight"
export _HINDSIGHT_DATA_DIR="$repo_root/.dev/data"

if [[ ! -x "$hs" ]]; then
    echo "error: $hs not found — run 'mise run build' first" >&2
    exit 1
fi

rm -rf "$_HINDSIGHT_DATA_DIR"

# run <session> <pwd> <exit> <cmd...> — record one finished command through
# the same start/end code paths the zsh hooks use.
run() {
    local session=$1 pwd=$2 exit_code=$3
    shift 3
    "$hs" start --session "$session" --pwd "$pwd" -- "$*"
    "$hs" end --session "$session" --exit "$exit_code"
}

proj="$HOME/repos/hindsight"

# seed-1: today, working in the project repo.
run seed-1 "$proj" 0 git status
run seed-1 "$proj" 0 cargo test
run seed-1 "$proj" 1 cargo build
run seed-1 "$proj" 1 git push

# seed-2: yesterday, same repo.
run seed-2 "$proj" 0 git status
run seed-2 "$proj" 0 ls -la
run seed-2 "$proj" 1 cargo test

# seed-3: a few days ago, scratch work in /tmp.
run seed-3 /tmp 0 curl -sO https://example.com/data.json
run seed-3 /tmp 0 ls -la
run seed-3 /tmp 0 git status

# seed-4: a month ago, in $HOME.
run seed-4 "$HOME" 0 ls -la
run seed-4 "$HOME" 127 htop
run seed-4 "$HOME" 0 git status

"$hs" fav add -- git status > /dev/null
"$hs" note set --note "Runs the full unit test suite in src/db.rs." -- cargo test > /dev/null

# Backdate sessions so they span days. start/end always record "now", and
# spreading sessions over time is what makes date labels and newest-first
# ordering in the session viewer testable. Needs the sqlite3 CLI; without it
# everything above still works, just with all sessions timestamped today.
if command -v sqlite3 > /dev/null; then
    db="$_HINDSIGHT_DATA_DIR/history.db"
    backdate() {
        sqlite3 "$db" "UPDATE commands SET start_ts = start_ts - $2, end_ts = end_ts - $2 WHERE session = '$1';"
    }
    backdate seed-2 86400      # 1 day
    backdate seed-3 259200     # 3 days
    backdate seed-4 2592000    # 30 days
else
    echo "warning: sqlite3 not found — sessions were not backdated, all timestamps are 'now'" >&2
fi

echo "Seeded dev DB at $_HINDSIGHT_DATA_DIR/history.db"
echo "Try: mise run dev -- stats"
echo "     mise run dev -- context json -- git status"
echo "     mise run dev:shell"
