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
# Rows are "<marker>\t<cmd>" (marker: ★ favorite, ✎ note); fzf matches on the
# command column only and the marker is stripped from the accepted line. A
# per-run state file holds the current view so fzf's reload knows what to show.
function __hindsight_search_widget() {
    emulate -L zsh
    local state selected
    state="${TMPDIR:-/tmp}/hindsight-picker.$$.$RANDOM"
    print -r -- history > "$state"
    selected="$(command hindsight picker --state "$state" --session "$__hindsight_session" \
        | fzf --height 60% --layout=reverse --scheme=history --delimiter=$'\t' --nth=2.. \
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
