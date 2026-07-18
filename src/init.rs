/// The zsh integration code, emitted by `hindsight init zsh`.
///
/// Add `eval "$(hindsight init zsh)"` to the END of your ~/.zshrc.
///
/// - `preexec` captures the command line, $PWD and a start time.
/// - `precmd` finalizes it with the exit code ($?).
/// - Ctrl-R opens fzf over the full history.
/// - Up/Down arrows do cwd-aware prefix search, cycling older/newer matches.
pub const ZSH: &str = r####"# hindsight — zsh integration. Add `eval "$(hindsight init zsh)"` to the end of ~/.zshrc.

# Per-shell session id: <epoch-ms>-<pid>. $$ is the shell PID; the millisecond
# start timestamp guards against PID reuse and makes ids sortable by start time.
# Terminal-agnostic: it depends only on the shell process, not on any terminal
# emulator or multiplexer. Assign only if unset: re-sourcing ~/.zshrc must NOT
# change the id mid-command, or the pending record written by preexec won't match
# the precmd that ends it.
if [[ -z "$__hindsight_session" ]]; then
    # Anonymous function: scopes the scratch var so it doesn't leak into the
    # interactive shell.
    () {
        zmodload zsh/datetime
        # ${EPOCHREALTIME%.*} = whole seconds, ${__frac[1,3]} = first 3 fractional
        # digits -> concatenated = milliseconds since the epoch (no subprocess).
        local __frac=${EPOCHREALTIME#*.}
        typeset -g __hindsight_session="${EPOCHREALTIME%.*}${__frac[1,3]}-$$"
    }
fi
# State for arrow-key history search. __hindsight_offset is how far back we've
# walked; __hindsight_prefix is the text typed before the first Up press, held
# fixed while cycling so matches stay consistent.
typeset -g __hindsight_offset=0
typeset -g __hindsight_prefix=""

# preexec: fires just before a command runs. $1 is the command line.
function __hindsight_preexec() {
    command hindsight start --session "$__hindsight_session" --pwd "$PWD" -- "$1"
}

# precmd: fires before each prompt, i.e. right after a command finished.
# Capture $? as the very first thing so nothing overwrites it.
function __hindsight_precmd() {
    local exit_code=$?
    __hindsight_offset=0
    __hindsight_prefix=""
    command hindsight end --session "$__hindsight_session" --exit "$exit_code"
}

typeset -ga preexec_functions precmd_functions
preexec_functions=("${(@)preexec_functions:#__hindsight_preexec}")
precmd_functions=("${(@)precmd_functions:#__hindsight_precmd}")
preexec_functions+=(__hindsight_preexec)
precmd_functions+=(__hindsight_precmd)

# --- Ctrl-R: fuzzy picker with two views (history / favorites) via fzf ---
#
# Ctrl-R opens the history view. Inside the picker:
#   - Ctrl-R again toggles between history and favorites-only views.
#   - Ctrl-S stars/unstars the highlighted command (creates favorites).
#   - Ctrl-E edits the highlighted command's note in $EDITOR.
#   - Ctrl-T shows/hides the note preview pane (hidden by default).
#   - Ctrl-O explores how the command was used across sessions (usage context).
# Records are NUL-terminated "<marker>\t<cmd>" (marker: ★ favorite, ✎ note);
# --read0 framing keeps multiline commands as a single fzf item, displayed
# across multiple list lines. fzf matches on the command column only and the
# marker (everything through the first tab) is stripped from the accepted
# item. A per-run state file holds the current view so fzf's reload knows
# what to show.
# NOTE: keep the fzf flags in sync with the BASH widget in this file.
function __hindsight_search_widget() {
    emulate -L zsh
    local state selected
    state="${TMPDIR:-/tmp}/hindsight-picker.$$.$RANDOM"
    print -r -- history > "$state"
    selected="$(command hindsight picker --state "$state" --session "$__hindsight_session" \
        | fzf --height 60% --layout=reverse --scheme=history --read0 --highlight-line \
              --delimiter=$'\t' --nth=2.. \
              --query "$LBUFFER" \
              --border=rounded --border-label=' hindsight ' --border-label-pos=2 \
              --header 'ctrl-r hist/fav   ctrl-s star   ctrl-e note   ctrl-t show note   ctrl-o context' \
              --header-first \
              --preview 'command hindsight note show -- {2..}' \
              --preview-window 'down,40%,wrap,border-top,hidden' \
              --preview-label ' note ' \
              --bind 'ctrl-t:toggle-preview' \
              --bind "ctrl-r:reload(command hindsight picker --state $state --session $__hindsight_session --toggle)" \
              --bind "ctrl-s:reload(command hindsight picker --state $state --session $__hindsight_session --star-toggle -- {2..})" \
              --bind "ctrl-e:execute(command hindsight note edit -- {2..})+reload(command hindsight picker --state $state --session $__hindsight_session)" \
              --bind "ctrl-o:execute(command hindsight context drill -- {2..})")"
    \command rm -f "$state"
    if [[ -n "$selected" ]]; then
        BUFFER="${selected#*$'\t'}"
        CURSOR=$#BUFFER
    fi
    zle reset-prompt
}
zle -N __hindsight_search_widget
bindkey '^R' __hindsight_search_widget

# --- Up/Down arrows: cwd-aware prefix search, cycling through matches ---
#
# On the first press we snapshot the typed text as the fixed search prefix.
# While cycling (previous widget was also ours) we keep that prefix and only
# move __hindsight_offset: Up walks to older matches, Down to newer. Reaching the
# top (offset 0) on Down restores exactly what was typed.
function __hindsight_history_search() {
    emulate -L zsh
    local dir=$1

    # Fresh search? (previous keystroke wasn't one of our widgets)
    if [[ "$LASTWIDGET" != __hindsight_up_widget && "$LASTWIDGET" != __hindsight_down_widget ]]; then
        __hindsight_prefix="$LBUFFER"
        __hindsight_offset=0
    fi

    if [[ "$dir" == up ]]; then
        (( __hindsight_offset++ ))
    else
        (( __hindsight_offset-- ))
        if (( __hindsight_offset <= 0 )); then
            # Back to (or above) the typed text: restore exactly what was typed.
            __hindsight_offset=0
            BUFFER="$__hindsight_prefix"
            CURSOR=$#BUFFER
            zle reset-prompt
            return
        fi
    fi

    # __hindsight_offset counts matches back from the typed text (0 = typed text,
    # 1 = newest match, ...), so the DB offset is one less.
    local match
    match="$(command hindsight search --cwd "$PWD" --offset "$(( __hindsight_offset - 1 ))" -- "$__hindsight_prefix")"
    if [[ -n "$match" ]]; then
        BUFFER="$match"
        CURSOR=$#BUFFER
    else
        # No match at this depth: clamp so we don't walk off the end.
        if [[ "$dir" == up ]]; then
            (( __hindsight_offset-- ))
        else
            (( __hindsight_offset++ ))
        fi
    fi
    zle reset-prompt
}

function __hindsight_up_widget()   { __hindsight_history_search up }
function __hindsight_down_widget() { __hindsight_history_search down }
zle -N __hindsight_up_widget
zle -N __hindsight_down_widget
bindkey '^[[A' __hindsight_up_widget     # Up arrow
bindkey '^[OA' __hindsight_up_widget     # Up arrow (application/cursor-key mode)
bindkey '^[[B' __hindsight_down_widget   # Down arrow
bindkey '^[OB' __hindsight_down_widget   # Down arrow (application/cursor-key mode)
"####;

/// The bash integration code, emitted by `hindsight init bash`.
///
/// Add `eval "$(hindsight init bash)"` to the END of your ~/.bashrc.
///
/// Feature parity with the zsh integration, built on bash primitives:
/// - a DEBUG trap plays the role of `preexec` (captures the typed line, $PWD
///   and a start time); `PROMPT_COMMAND` plays the role of `precmd` (finalizes
///   with the exit code). If bash-preexec is already loaded, its
///   `preexec_functions`/`precmd_functions` are used instead.
/// - Ctrl-R opens fzf over the full history (readline `bind -x`).
/// - Up/Down arrows do cwd-aware prefix search, cycling older/newer matches.
///
/// Requires bash >= 5.0 (EPOCHREALTIME, reliable READLINE_POINT writes); the
/// script degrades to a no-op with a warning on older bash.
pub const BASH: &str = r####"# hindsight — bash integration. Add `eval "$(hindsight init bash)"` to the end of ~/.bashrc.

# Everything lives inside one guard block: this code is eval'd from ~/.bashrc,
# where a top-level `return` would return from ~/.bashrc itself and skip the
# rest of the user's config.
if [[ $- != *i* ]]; then
    :  # non-interactive shell: nothing to do
elif ((BASH_VERSINFO[0] < 5)); then
    echo "hindsight: bash >= 5.0 required (found $BASH_VERSION); integration not loaded" >&2
else

# Per-shell session id: <epoch-ms>-<pid>. $$ is the shell PID; the millisecond
# start timestamp guards against PID reuse and makes ids sortable by start time.
# Terminal-agnostic: it depends only on the shell process, not on any terminal
# emulator or multiplexer. Assign only if unset: re-sourcing ~/.bashrc must NOT
# change the id mid-command, or the pending record written at preexec time
# won't match the precmd that ends it.
if [[ -z "${__hindsight_session-}" ]]; then
    # EPOCHREALTIME is "<seconds><decimal-point><microseconds>" where the
    # decimal point is locale-dependent (`.` or `,`). Capture once so seconds
    # and fraction come from the same instant.
    __hindsight_now=$EPOCHREALTIME
    __hindsight_frac=${__hindsight_now#*[.,]}
    __hindsight_session="${__hindsight_now%[.,]*}${__hindsight_frac:0:3}-$$"
    unset __hindsight_now __hindsight_frac
fi

# State for arrow-key history search. __hindsight_offset is how far back we've
# walked; __hindsight_prefix is the text typed before the first Up press, held
# fixed while cycling so matches stay consistent. __hindsight_last_line is the
# buffer text our arrow widgets last produced — bash has no $LASTWIDGET, so a
# changed buffer is how we detect that the user typed in between (fresh search).
__hindsight_offset=0
__hindsight_prefix=""
__hindsight_last_line=""
# Recording state: __hindsight_at_prompt gates the DEBUG trap so only the FIRST
# top-level command of each input line fires `hindsight start` (pipelines and
# loops are many DEBUG hits but one history entry); __hindsight_pending tells
# precmd whether there is a started command to end; __hindsight_histno is the
# history number snapshotted at the prompt, used to detect whether Enter
# actually submitted a new command (empty Enter adds no history entry).
__hindsight_at_prompt=0
__hindsight_pending=0
__hindsight_histno=""

# The typed line is recovered from history (bash's $BASH_COMMAND is the current
# simple command, not the full line). cmdhist+lithist store a multiline command
# as ONE history entry with embedded newlines, so it round-trips verbatim.
# Recording requires history to be enabled with HISTSIZE > 0 (the interactive
# default). Caveat: HISTCONTROL still applies — with `ignoredups` an
# immediately repeated command adds no history entry and is not re-recorded,
# and with `ignorespace` space-prefixed commands are not recorded (consistent
# with bash's own notion of what is history-worthy).
shopt -s cmdhist lithist

# precmd: runs before each prompt, i.e. right after a command finished.
# Capture $? as the very first thing so nothing overwrites it.
__hindsight_precmd() {
    local exit_code=$?
    if ((__hindsight_pending)); then
        __hindsight_pending=0
        command hindsight end --session "$__hindsight_session" --exit "$exit_code"
    fi
    __hindsight_offset=0
    __hindsight_prefix=""
    __hindsight_last_line=""
    local histline
    histline=$(HISTTIMEFORMAT= builtin history 1)
    builtin read -r __hindsight_histno _ <<< "$histline"
    __hindsight_histno=${__hindsight_histno%\*}
    __hindsight_at_prompt=1
}

# preexec stand-in: the DEBUG trap fires before every top-level command.
# Record only when (a) we're not in a completion, (b) this is the first
# command since the prompt, and (c) Enter actually added a history entry
# (the history number advanced past precmd's snapshot — filters empty Enter
# and other PROMPT_COMMAND hooks, which add no history). Functions do not
# inherit the DEBUG trap by default, so our own helpers don't re-trigger it.
__hindsight_debug() {
    [[ -n "${COMP_LINE-}" ]] && return
    ((__hindsight_at_prompt)) || return
    local histline histno cmd
    histline=$(HISTTIMEFORMAT= builtin history 1)
    builtin read -r histno _ <<< "$histline"
    histno=${histno%\*}
    [[ "$histno" == "$__hindsight_histno" ]] && return
    __hindsight_at_prompt=0
    __hindsight_pending=1
    # Strip the leading " NNN  " (or " NNN* ") from the first line only; later
    # lines of a multiline entry are the command text itself.
    cmd=$(sed '1 s/^ *[0-9][0-9]*[* ] //' <<< "$histline")
    command hindsight start --session "$__hindsight_session" --pwd "$PWD" -- "$cmd"
}

if [[ -n "${bash_preexec_imported:-${__bp_imported:-}}" ]]; then
    # bash-preexec is loaded: it owns the DEBUG trap and hands the full typed
    # line to preexec functions as $1 (exactly like zsh) — register into its
    # arrays instead of competing for the trap.
    __hindsight_preexec() {
        __hindsight_pending=1
        command hindsight start --session "$__hindsight_session" --pwd "$PWD" -- "$1"
    }
    if [[ " ${preexec_functions[*]-} " != *" __hindsight_preexec "* ]]; then
        preexec_functions+=(__hindsight_preexec)
    fi
    if [[ " ${precmd_functions[*]-} " != *" __hindsight_precmd "* ]]; then
        precmd_functions+=(__hindsight_precmd)
    fi
elif [[ -n "$(trap -p DEBUG)" && "$(trap -p DEBUG)" != *__hindsight_debug* ]]; then
    # Someone else already instruments DEBUG; never clobber it. Search over
    # previously recorded history still works — only recording is off.
    echo "hindsight: a DEBUG trap is already installed; command recording disabled (consider loading bash-preexec before hindsight)" >&2
else
    trap '__hindsight_debug' DEBUG
    # Prepend so we capture $? before other prompt hooks run. PROMPT_COMMAND
    # may be an array (bash >= 5.1) or a string.
    if [[ "$(declare -p PROMPT_COMMAND 2>/dev/null)" == "declare -a"* ]]; then
        if [[ " ${PROMPT_COMMAND[*]} " != *" __hindsight_precmd "* ]]; then
            PROMPT_COMMAND=(__hindsight_precmd "${PROMPT_COMMAND[@]}")
        fi
    elif [[ "${PROMPT_COMMAND-}" != *__hindsight_precmd* ]]; then
        PROMPT_COMMAND="__hindsight_precmd${PROMPT_COMMAND:+;$PROMPT_COMMAND}"
    fi
fi

# --- Ctrl-R: fuzzy picker with two views (history / favorites) via fzf ---
#
# Ctrl-R opens the history view. Inside the picker:
#   - Ctrl-R again toggles between history and favorites-only views.
#   - Ctrl-S stars/unstars the highlighted command (creates favorites).
#   - Ctrl-E edits the highlighted command's note in $EDITOR.
#   - Ctrl-T shows/hides the note preview pane (hidden by default).
#   - Ctrl-O explores how the command was used across sessions (usage context).
# Records are NUL-terminated "<marker>\t<cmd>" (marker: ★ favorite, ✎ note);
# --read0 framing keeps multiline commands as a single fzf item, displayed
# across multiple list lines. fzf matches on the command column only and the
# marker (everything through the first tab) is stripped from the accepted
# item. A per-run state file holds the current view so fzf's reload knows
# what to show. On accept the command lands in the edit buffer (READLINE_LINE)
# without running; readline redraws by itself after a bind -x handler.
# NOTE: keep the fzf flags in sync with the ZSH widget in this file.
__hindsight_search_widget() {
    local state selected
    state="${TMPDIR:-/tmp}/hindsight-picker.$$.$RANDOM"
    printf 'history\n' > "$state"
    selected="$(command hindsight picker --state "$state" --session "$__hindsight_session" \
        | fzf --height 60% --layout=reverse --scheme=history --read0 --highlight-line \
              --delimiter=$'\t' --nth=2.. \
              --query "${READLINE_LINE:0:READLINE_POINT}" \
              --border=rounded --border-label=' hindsight ' --border-label-pos=2 \
              --header 'ctrl-r hist/fav   ctrl-s star   ctrl-e note   ctrl-t show note   ctrl-o context' \
              --header-first \
              --preview 'command hindsight note show -- {2..}' \
              --preview-window 'down,40%,wrap,border-top,hidden' \
              --preview-label ' note ' \
              --bind 'ctrl-t:toggle-preview' \
              --bind "ctrl-r:reload(command hindsight picker --state $state --session $__hindsight_session --toggle)" \
              --bind "ctrl-s:reload(command hindsight picker --state $state --session $__hindsight_session --star-toggle -- {2..})" \
              --bind "ctrl-e:execute(command hindsight note edit -- {2..})+reload(command hindsight picker --state $state --session $__hindsight_session)" \
              --bind "ctrl-o:execute(command hindsight context drill -- {2..})")"
    command rm -f "$state"
    if [[ -n "$selected" ]]; then
        READLINE_LINE="${selected#*$'\t'}"
        READLINE_POINT=${#READLINE_LINE}
    fi
}
bind -m emacs      -x '"\C-r": __hindsight_search_widget'
bind -m vi-insert  -x '"\C-r": __hindsight_search_widget'
bind -m vi-command -x '"\C-r": __hindsight_search_widget'

# --- Up/Down arrows: cwd-aware prefix search, cycling through matches ---
#
# On the first press we snapshot the typed text as the fixed search prefix.
# While cycling (the buffer still holds exactly what our widget last wrote —
# the $LASTWIDGET stand-in) we keep that prefix and only move
# __hindsight_offset: Up walks to older matches, Down to newer. Reaching the
# top (offset 0) on Down restores exactly what was typed. This replaces
# readline's native arrow history navigation (the same trade zsh users make);
# rebind the arrows after the eval line to opt out.
__hindsight_history_search() {
    local dir=$1 match

    # Fresh search? (the buffer changed since our widgets last wrote it)
    if [[ -z "$__hindsight_last_line" || "$READLINE_LINE" != "$__hindsight_last_line" ]]; then
        __hindsight_prefix="${READLINE_LINE:0:READLINE_POINT}"
        __hindsight_offset=0
    fi

    if [[ "$dir" == up ]]; then
        ((__hindsight_offset++))
    else
        ((__hindsight_offset--))
        if ((__hindsight_offset <= 0)); then
            # Back to (or above) the typed text: restore exactly what was typed.
            __hindsight_offset=0
            READLINE_LINE="$__hindsight_prefix"
            READLINE_POINT=${#READLINE_LINE}
            __hindsight_last_line="$READLINE_LINE"
            return
        fi
    fi

    # __hindsight_offset counts matches back from the typed text (0 = typed text,
    # 1 = newest match, ...), so the DB offset is one less.
    match="$(command hindsight search --cwd "$PWD" --offset "$((__hindsight_offset - 1))" -- "$__hindsight_prefix")"
    if [[ -n "$match" ]]; then
        READLINE_LINE="$match"
        READLINE_POINT=${#READLINE_LINE}
    else
        # No match at this depth: clamp so we don't walk off the end.
        if [[ "$dir" == up ]]; then
            ((__hindsight_offset--))
        else
            ((__hindsight_offset++))
        fi
    fi
    __hindsight_last_line="$READLINE_LINE"
}
__hindsight_up_widget()   { __hindsight_history_search up; }
__hindsight_down_widget() { __hindsight_history_search down; }
bind -m emacs     -x '"\e[A": __hindsight_up_widget'     # Up arrow
bind -m emacs     -x '"\eOA": __hindsight_up_widget'     # Up arrow (application/cursor-key mode)
bind -m emacs     -x '"\e[B": __hindsight_down_widget'   # Down arrow
bind -m emacs     -x '"\eOB": __hindsight_down_widget'   # Down arrow (application/cursor-key mode)
bind -m vi-insert -x '"\e[A": __hindsight_up_widget'
bind -m vi-insert -x '"\eOA": __hindsight_up_widget'
bind -m vi-insert -x '"\e[B": __hindsight_down_widget'
bind -m vi-insert -x '"\eOB": __hindsight_down_widget'

fi  # end interactive + bash >= 5 guard
"####;

#[cfg(test)]
mod tests {
    use std::io::Write;
    use std::process::{Command, Stdio};

    /// Pipe a script through `<shell> -n` (parse only). Skips silently when
    /// the shell binary isn't installed, so `cargo test` works everywhere.
    fn check_syntax(shell: &str, script: &str) {
        let spawned = Command::new(shell)
            .arg("-n")
            .stdin(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn();
        let Ok(mut child) = spawned else {
            eprintln!("skipping: {shell} not installed");
            return;
        };
        child
            .stdin
            .take()
            .unwrap()
            .write_all(script.as_bytes())
            .unwrap();
        let out = child.wait_with_output().unwrap();
        assert!(
            out.status.success(),
            "{shell} -n rejected the emitted script:\n{}",
            String::from_utf8_lossy(&out.stderr)
        );
    }

    #[test]
    fn zsh_script_parses() {
        check_syntax("zsh", super::ZSH);
    }

    #[test]
    fn bash_script_parses() {
        check_syntax("bash", super::BASH);
    }
}
