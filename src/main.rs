mod config;
mod db;
mod init;

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(
    name = "hindsight",
    version,
    about = "A fast command history recorder and search tool for zsh",
    long_about = "hindsight records every command you run in zsh — with its working directory, exit \
code, and timestamps — into a local SQLite database, and makes it easy to search, favorite, \
annotate, and analyze your shell history.\n\n\
FOR AI AGENTS: two read-only commands emit machine-readable JSON on stdout:\n\
  hindsight inspect -- <command>   All metadata for ONE command: run_count, exit_codes\n\
                                (code->count), directories (cwd->count), note,\n\
                                is_favorite, first_run/last_run (unix seconds).\n\
  hindsight stats [--limit N]      Global aggregates: top_commands, error_prone,\n\
                                top_directories, and totals.\n\
Pass a command verbatim after `--` (e.g. `hindsight inspect -- git push`). Run any\n\
subcommand with --help for its exact JSON shape."
)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Generate shell integration code (currently: zsh).
    Init {
        /// Shell to generate configuration for.
        shell: String,
    },
    /// Record a pending command (called from the zsh preexec hook).
    Start {
        /// Per-shell session id.
        #[arg(long)]
        session: String,
        /// Working directory the command runs in.
        #[arg(long)]
        pwd: String,
        /// The command line (everything after `--`).
        #[arg(last = true, required = true, num_args = 1..)]
        cmd: Vec<String>,
    },
    /// Finalize the pending command with its exit code (called from precmd).
    End {
        #[arg(long)]
        session: String,
        #[arg(long)]
        exit: i64,
    },
    /// List stored commands (newest first, deduped).
    Query {
        /// List all matching commands.
        #[arg(short, long)]
        list: bool,
        /// Only commands run in this directory.
        #[arg(long)]
        cwd: Option<String>,
        /// Only commands with this exit code.
        #[arg(long)]
        exit: Option<i64>,
        /// Maximum number of results.
        #[arg(long)]
        limit: Option<i64>,
    },
    /// cwd-aware prefix search (called from the Up-arrow widget).
    Search {
        #[arg(long)]
        cwd: String,
        /// Which match to return (0 = best/newest), for repeated Up presses.
        #[arg(long, default_value_t = 0)]
        offset: i64,
        /// The typed prefix (everything after `--`; may be empty).
        #[arg(last = true, num_args = 0..)]
        prefix: Vec<String>,
    },
    /// Import commands from an existing zsh history file.
    Import {
        /// Path to the history file (default: ~/.zsh_history).
        #[arg(long)]
        from: Option<String>,
    },
    /// Manage favorite commands.
    Fav {
        #[command(subcommand)]
        action: FavAction,
    },
    /// Manage notes attached to commands.
    Note {
        #[command(subcommand)]
        action: NoteAction,
    },
    /// Hide a command from all views (soft delete; data is never destroyed).
    #[command(long_about = "Soft-delete a command: hide it from every user-facing view (picker, \
query, arrow search, favorites, inspect, stats). NOTHING is physically removed — the data stays in \
the database and can be inspected with `hindsight deleted` or brought back with `hindsight restore`.")]
    Delete {
        #[arg(last = true, required = true, num_args = 1..)]
        cmd: Vec<String>,
    },
    /// Restore (un-hide) a soft-deleted command.
    Restore {
        #[arg(last = true, required = true, num_args = 1..)]
        cmd: Vec<String>,
    },
    /// Soft-delete stored commands matching the ignore list (dry run by default).
    Prune {
        #[command(subcommand)]
        what: PruneWhat,
    },
    /// Inspect soft-deleted commands, as JSON (for AI agents).
    Deleted {
        #[command(subcommand)]
        action: DeletedAction,
    },
    /// Inspect ALL metadata for one command, as JSON (for AI agents).
    #[command(long_about = "Inspect ALL metadata for one command, as JSON (for AI agents).\n\n\
Prints a single JSON object to stdout:\n\
  command       the command inspected (string)\n\
  found         false if the command is unknown (then no other fields)\n\
  run_count     how many times it has run (integer)\n\
  exit_codes    object mapping exit code -> count; key \"null\" = never finished\n\
  directories   array of {\"cwd\", \"count\"}, most-used directory first\n\
  note          the attached note, or null\n\
  is_favorite   boolean\n\
  first_run     unix seconds of the earliest run, or null\n\
  last_run      unix seconds of the most recent run, or null\n\
Pass the command verbatim after `--`, e.g.  hindsight inspect -- git push")]
    Inspect {
        #[arg(last = true, required = true, num_args = 1..)]
        cmd: Vec<String>,
    },
    /// Global usage statistics across all history, as JSON (for AI agents).
    #[command(long_about = "Global usage statistics across all history, as JSON (for AI agents).\n\n\
Prints a single JSON object to stdout:\n\
  totals          {distinct_commands, total_runs, favorites, notes}\n\
  top_commands    array of {\"command\", \"count\"}, most-run first\n\
  error_prone     array of {\"command\", \"failures\", \"runs\"} (>=1 non-zero exit)\n\
  top_directories array of {\"cwd\", \"count\"}, most-used first\n\
Use --limit to cap each ranked list (default 20).")]
    Stats {
        /// Max entries in each ranked list (default 20).
        #[arg(long, default_value_t = 20)]
        limit: i64,
    },
    /// Explore how a command was used across sessions (timelines, JSON for agents).
    Context {
        #[command(subcommand)]
        action: ContextAction,
    },
    /// Drive the fzf picker (history/favorites views). Called from the widget.
    Picker {
        /// Path to the per-invocation mode state file.
        #[arg(long)]
        state: String,
        /// Flip the view mode (history <-> favorites) before printing.
        #[arg(long)]
        toggle: bool,
        /// Star/unstar this command, then reprint the current view.
        #[arg(long)]
        star_toggle: bool,
        /// The command to star/unstar (everything after `--`).
        #[arg(last = true, num_args = 0..)]
        cmd: Vec<String>,
    },
}

#[derive(Subcommand)]
enum FavAction {
    /// Add a command to favorites.
    Add {
        #[arg(last = true, required = true, num_args = 1..)]
        cmd: Vec<String>,
    },
    /// Remove a command from favorites.
    Rm {
        #[arg(last = true, required = true, num_args = 1..)]
        cmd: Vec<String>,
    },
    /// Toggle a command's favorite state.
    Toggle {
        #[arg(last = true, required = true, num_args = 1..)]
        cmd: Vec<String>,
    },
    /// List favorites, newest first.
    List,
}

#[derive(Subcommand)]
enum NoteAction {
    /// Print a command's note (or a placeholder if none). Drives the preview pane.
    Show {
        #[arg(last = true, required = true, num_args = 1..)]
        cmd: Vec<String>,
    },
    /// Open the note in $EDITOR to add or change it.
    Edit {
        #[arg(last = true, required = true, num_args = 1..)]
        cmd: Vec<String>,
    },
    /// Set a note non-interactively.
    Set {
        #[arg(long)]
        note: String,
        #[arg(last = true, required = true, num_args = 1..)]
        cmd: Vec<String>,
    },
    /// Clear a command's note.
    Clear {
        #[arg(last = true, required = true, num_args = 1..)]
        cmd: Vec<String>,
    },
}

#[derive(Subcommand)]
enum PruneWhat {
    /// Soft-delete stored commands matching the ignore list in hindsight.toml.
    #[command(long_about = "Soft-delete stored commands that match the ignore list in hindsight.toml.\n\n\
DRY RUN BY DEFAULT: prints the commands (and how many history rows each) that WOULD be soft-deleted \
and changes nothing. Re-run with --apply to actually hide them. Soft delete never destroys data — \
hidden commands remain in the database and can be seen with `hindsight deleted` or restored with \
`hindsight restore`.")]
    Ignore {
        /// Actually soft-delete the matches (default is a dry run that only prints).
        #[arg(long)]
        apply: bool,
    },
}

#[derive(Subcommand)]
enum DeletedAction {
    /// List soft-deleted commands as JSON: [{command, deleted_ts, run_count}].
    List,
    /// Inspect one soft-deleted command as JSON (same shape as `inspect`, plus deleted/deleted_ts).
    Inspect {
        #[arg(last = true, required = true, num_args = 1..)]
        cmd: Vec<String>,
    },
}

#[derive(Subcommand)]
enum ContextAction {
    /// All sessions a command ran in, with whole-session timelines, as JSON (for AI agents).
    #[command(long_about = "Show how a command was used across sessions, as JSON (for AI agents).\n\n\
Prints a single JSON object to stdout:\n\
  command    the command inspected (string)\n\
  found      false if the command has no active occurrences\n\
  sessions   array, newest first, of:\n\
    session    opaque session id\n\
    label      human label (e.g. \"~/repo\")\n\
    count      times the command ran in this session\n\
    timeline   the whole session, ordered: [{cmd, exit_code, start_ts, is_match}]\n\
Pass the command verbatim after `--`.")]
    Json {
        #[arg(last = true, required = true, num_args = 1..)]
        cmd: Vec<String>,
    },
    /// Internal: TSV `session<TAB>label<TAB>count` for the drill picker.
    Sessions {
        #[arg(last = true, required = true, num_args = 1..)]
        cmd: Vec<String>,
    },
    /// Internal: text timeline of a session, the matched command highlighted (preview pane).
    Timeline {
        #[arg(long)]
        session: String,
        #[arg(last = true, num_args = 0..)]
        cmd: Vec<String>,
    },
    /// Open a session's full timeline in $EDITOR (for copy-paste).
    Edit {
        #[arg(long)]
        session: String,
    },
    /// Interactive: pick a session and preview its timeline (bound to Ctrl-O in the picker).
    Drill {
        #[arg(last = true, required = true, num_args = 1..)]
        cmd: Vec<String>,
    },
}

/// Build the JSON object for a command's full metadata (shared by `inspect` and
/// `deleted inspect`). `extra` fields (e.g. deleted/deleted_ts) are merged in.
fn command_stats_json(cmd: &str, stats: &db::CommandStats) -> serde_json::Value {
    let exit_codes: serde_json::Map<String, serde_json::Value> = stats
        .exit_codes
        .iter()
        .map(|(code, count)| {
            let key = code.map(|c| c.to_string()).unwrap_or_else(|| "null".into());
            (key, serde_json::json!(count))
        })
        .collect();
    let directories: Vec<serde_json::Value> = stats
        .directories
        .iter()
        .map(|(cwd, count)| serde_json::json!({ "cwd": cwd, "count": count }))
        .collect();
    serde_json::json!({
        "command": cmd,
        "found": true,
        "run_count": stats.run_count,
        "exit_codes": exit_codes,
        "directories": directories,
        "note": if stats.note.is_empty() { serde_json::Value::Null } else { serde_json::json!(stats.note) },
        "is_favorite": stats.is_favorite,
        "first_run": stats.first_run,
        "last_run": stats.last_run,
    })
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Commands::Init { shell } => match shell.as_str() {
            "zsh" => print!("{}", init::ZSH),
            other => {
                anyhow::bail!("unsupported shell '{other}' (only 'zsh' is supported)");
            }
        },
        Commands::Start { session, pwd, cmd } => {
            // Fail-open: nothing in the hook path (bad config, DB trouble) may
            // break the prompt — warn in one line on stderr and exit 0.
            let record = || -> Result<()> {
                let conn = db::open()?;
                let ignore = config::ignore_regexes_fail_open();
                db::start(&conn, &session, &pwd, &cmd.join(" "), &ignore)
            };
            if let Err(e) = record() {
                eprintln!("hindsight: command not recorded: {e:#}");
            }
        }
        Commands::End { session, exit } => {
            let finalize = || -> Result<()> {
                let conn = db::open()?;
                db::end(&conn, &session, exit)
            };
            if let Err(e) = finalize() {
                eprintln!("hindsight: exit code not recorded: {e:#}");
            }
        }
        Commands::Query {
            list: _,
            cwd,
            exit,
            limit,
        } => {
            // `--list` is accepted for symmetry with future query modes; listing
            // is currently the only mode.
            let conn = db::open()?;
            for cmd in db::list(&conn, cwd.as_deref(), exit, limit)? {
                println!("{cmd}");
            }
        }
        Commands::Search {
            cwd,
            offset,
            prefix,
        } => {
            let conn = db::open()?;
            let prefix = prefix.join(" ");
            if let Some(m) = db::search(&conn, &cwd, &prefix, offset)? {
                println!("{m}");
            }
        }
        Commands::Import { from } => {
            let path = match from {
                Some(p) => std::path::PathBuf::from(p),
                None => dirs::home_dir()
                    .map(|h| h.join(".zsh_history"))
                    .ok_or_else(|| anyhow::anyhow!("could not resolve home directory"))?,
            };
            let conn = db::open()?;
            let n = db::import_zsh(&conn, &path)?;
            eprintln!("hindsight: imported {n} commands from {}", path.display());
        }
        Commands::Fav { action } => {
            let conn = db::open()?;
            match action {
                FavAction::Add { cmd } => db::fav_add(&conn, &cmd.join(" "))?,
                FavAction::Rm { cmd } => db::fav_remove(&conn, &cmd.join(" "))?,
                FavAction::Toggle { cmd } => {
                    let cmd = cmd.join(" ");
                    if db::fav_toggle(&conn, &cmd)? {
                        println!("★ favorited: {cmd}");
                    } else {
                        println!("  unfavorited: {cmd}");
                    }
                }
                FavAction::List => {
                    for cmd in db::fav_list(&conn)? {
                        println!("{cmd}");
                    }
                }
            }
        }
        Commands::Note { action } => {
            let conn = db::open()?;
            match action {
                NoteAction::Show { cmd } => {
                    let note = db::note_get(&conn, &cmd.join(" "))?;
                    if note.is_empty() {
                        println!("(no note — press ctrl-e to add one)");
                    } else {
                        println!("{note}");
                    }
                }
                NoteAction::Edit { cmd } => note_edit(&conn, &cmd.join(" "))?,
                NoteAction::Set { note, cmd } => db::note_set(&conn, &cmd.join(" "), &note)?,
                NoteAction::Clear { cmd } => db::note_set(&conn, &cmd.join(" "), "")?,
            }
        }
        Commands::Delete { cmd } => {
            let conn = db::open()?;
            db::soft_delete(&conn, &cmd.join(" "))?;
        }
        Commands::Restore { cmd } => {
            let conn = db::open()?;
            let cmd = cmd.join(" ");
            db::restore(&conn, &cmd)?;
            println!("restored: {cmd}");
        }
        Commands::Prune { what } => {
            let PruneWhat::Ignore { apply } = what;
            let conn = db::open()?;
            // Explicit user command: hard-error on a bad pattern.
            let patterns = config::load_ignore()?;
            let (ignore, bad) = config::compile(&patterns);
            if let Some((pat, err)) = bad.first() {
                anyhow::bail!("invalid ignore pattern {pat:?}: {err}");
            }
            let matched = db::prune_ignored(&conn, &ignore, apply)?;
            let rows: i64 = matched.iter().map(|(_, n)| n).sum();
            if matched.is_empty() {
                println!("Nothing matches the ignore list.");
            } else if apply {
                println!("Soft-deleted {} commands ({rows} history rows).", matched.len());
            } else {
                println!(
                    "DRY RUN — nothing deleted. {} commands ({rows} history rows) would be soft-deleted:",
                    matched.len()
                );
                for (cmd, n) in &matched {
                    println!("  {n}×  {cmd}");
                }
                println!("Re-run with --apply to soft-delete them.");
            }
        }
        Commands::Deleted { action } => {
            let conn = db::open()?;
            match action {
                DeletedAction::List => {
                    let items: Vec<serde_json::Value> = db::deleted_list(&conn)?
                        .iter()
                        .map(|(cmd, ts, runs)| {
                            serde_json::json!({ "command": cmd, "deleted_ts": ts, "run_count": runs })
                        })
                        .collect();
                    println!("{}", serde_json::to_string_pretty(&items)?);
                }
                DeletedAction::Inspect { cmd } => {
                    let cmd = cmd.join(" ");
                    let del_ts = db::deleted_ts(&conn, &cmd)?;
                    let value = match db::inspect_any(&conn, &cmd)? {
                        None => serde_json::json!({ "command": cmd, "found": false }),
                        Some(s) => {
                            let mut v = command_stats_json(&cmd, &s);
                            let obj = v.as_object_mut().unwrap();
                            obj.insert("deleted".into(), serde_json::json!(del_ts.is_some()));
                            obj.insert("deleted_ts".into(), serde_json::json!(del_ts));
                            v
                        }
                    };
                    println!("{}", serde_json::to_string_pretty(&value)?);
                }
            }
        }
        Commands::Inspect { cmd } => {
            let conn = db::open()?;
            let cmd = cmd.join(" ");
            let value = match db::inspect(&conn, &cmd)? {
                None => serde_json::json!({ "command": cmd, "found": false }),
                Some(s) => command_stats_json(&cmd, &s),
            };
            println!("{}", serde_json::to_string_pretty(&value)?);
        }
        Commands::Stats { limit } => {
            let conn = db::open()?;
            let totals = db::stats_totals(&conn)?;
            let top_commands: Vec<serde_json::Value> = db::stats_top_commands(&conn, limit)?
                .iter()
                .map(|(cmd, count)| serde_json::json!({ "command": cmd, "count": count }))
                .collect();
            let error_prone: Vec<serde_json::Value> = db::stats_error_prone(&conn, limit)?
                .iter()
                .map(|(cmd, fails, total)| {
                    serde_json::json!({ "command": cmd, "failures": fails, "runs": total })
                })
                .collect();
            let top_directories: Vec<serde_json::Value> = db::stats_top_dirs(&conn, limit)?
                .iter()
                .map(|(cwd, count)| serde_json::json!({ "cwd": cwd, "count": count }))
                .collect();
            let value = serde_json::json!({
                "totals": {
                    "distinct_commands": totals.distinct_commands,
                    "total_runs": totals.total_runs,
                    "favorites": totals.favorites,
                    "notes": totals.notes,
                },
                "top_commands": top_commands,
                "error_prone": error_prone,
                "top_directories": top_directories,
            });
            println!("{}", serde_json::to_string_pretty(&value)?);
        }
        Commands::Context { action } => {
            let conn = db::open()?;
            match action {
                ContextAction::Json { cmd } => {
                    let cmd = cmd.join(" ");
                    let sessions = db::context_sessions(&conn, &cmd)?;
                    if sessions.is_empty() {
                        let v = serde_json::json!({ "command": cmd, "found": false });
                        println!("{}", serde_json::to_string_pretty(&v)?);
                    } else {
                        let arr: Vec<serde_json::Value> = sessions
                            .iter()
                            .map(|s| {
                                let timeline: Vec<serde_json::Value> = db::session_timeline(&conn, &s.session)
                                    .unwrap_or_default()
                                    .iter()
                                    .map(|t| {
                                        serde_json::json!({
                                            "cmd": t.cmd,
                                            "exit_code": t.exit_code,
                                            "start_ts": t.start_ts,
                                            "is_match": t.cmd == cmd,
                                        })
                                    })
                                    .collect();
                                serde_json::json!({
                                    "session": s.session,
                                    "label": s.label,
                                    "count": s.count,
                                    "timeline": timeline,
                                })
                            })
                            .collect();
                        let v = serde_json::json!({ "command": cmd, "found": true, "sessions": arr });
                        println!("{}", serde_json::to_string_pretty(&v)?);
                    }
                }
                ContextAction::Sessions { cmd } => {
                    let cmd = cmd.join(" ");
                    for s in db::context_sessions(&conn, &cmd)? {
                        println!("{}\t{}\t{}", s.session, s.label, s.count);
                    }
                }
                ContextAction::Timeline { session, cmd } => {
                    let cmd = cmd.join(" ");
                    print!("{}", render_timeline(&conn, &session, &cmd)?);
                }
                ContextAction::Edit { session } => {
                    // Whole-session timeline (no specific match highlight needed).
                    let text = render_timeline(&conn, &session, "")?;
                    open_in_editor(&text, "session")?;
                }
                ContextAction::Drill { cmd } => {
                    let cmd = cmd.join(" ");
                    context_drill(&conn, &cmd)?;
                }
            }
        }
        Commands::Picker {
            state,
            toggle,
            star_toggle,
            cmd,
        } => {
            let conn = db::open()?;
            picker(&conn, &state, toggle, star_toggle, &cmd.join(" "))?;
        }
    }
    Ok(())
}

/// Open `initial` text in $EDITOR and return the final file contents.
///
/// hindsight is a terminal tool: it honors $EDITOR only (not $VISUAL), with no
/// fallback — errors if $EDITOR is unset. The editor is resolved before any
/// temp file is created, so an unset $EDITOR leaves nothing behind. `tag` names
/// the temp file (e.g. "note", "session").
fn open_in_editor(initial: &str, tag: &str) -> Result<String> {
    use std::io::Write;

    let editor = match std::env::var("EDITOR") {
        Ok(e) if !e.trim().is_empty() => e,
        _ => anyhow::bail!("$EDITOR is not set; set it (e.g. `export EDITOR=hx`)"),
    };

    // Notes can be sensitive, and the shared temp dir is world-writable: prefer
    // the per-user $XDG_RUNTIME_DIR, create the file exclusively (O_EXCL, so a
    // pre-planted symlink can't redirect the write), and keep it owner-only.
    let dir = std::env::var("XDG_RUNTIME_DIR")
        .ok()
        .filter(|d| !d.is_empty())
        .map(std::path::PathBuf::from)
        .unwrap_or_else(std::env::temp_dir);
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.subsec_nanos())
        .unwrap_or(0);
    let tmp = dir.join(format!("hindsight-{tag}-{}-{nonce}.txt", std::process::id()));
    {
        let mut opts = std::fs::OpenOptions::new();
        opts.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            opts.mode(0o600);
        }
        let mut f = opts
            .open(&tmp)
            .with_context(|| format!("creating {}", tmp.display()))?;
        f.write_all(initial.as_bytes())?;
    }

    // Split so values like `code --wait` work.
    let mut parts = editor.split_whitespace();
    let prog = parts.next().unwrap_or("vi");
    let status = std::process::Command::new(prog).args(parts).arg(&tmp).status()?;
    if !status.success() {
        let _ = std::fs::remove_file(&tmp);
        anyhow::bail!("editor exited with an error");
    }

    let edited = std::fs::read_to_string(&tmp)?;
    let _ = std::fs::remove_file(&tmp);
    Ok(edited)
}

/// Open a command's note in $EDITOR (errors if $EDITOR is unset), then save it back.
fn note_edit(conn: &rusqlite::Connection, cmd: &str) -> Result<()> {
    let current = db::note_get(conn, cmd)?;
    let edited = open_in_editor(&current, "note")?;
    db::note_set(conn, cmd, edited.trim_end())?;
    Ok(())
}

/// Render a session's timeline as text for the preview pane / $EDITOR. Rows whose
/// command equals `target` (when non-empty) are prefixed `→` with their exit code.
fn render_timeline(conn: &rusqlite::Connection, session: &str, target: &str) -> Result<String> {
    let rows = db::session_timeline(conn, session)?;
    let mut out = String::new();
    for t in &rows {
        if !target.is_empty() && t.cmd == target {
            let exit = t
                .exit_code
                .map(|c| format!("(exit {c})"))
                .unwrap_or_else(|| "(no exit)".into());
            out.push_str(&format!("→ {}   {}\n", t.cmd, exit));
        } else {
            out.push_str(&format!("  {}\n", t.cmd));
        }
    }
    if out.is_empty() {
        out.push_str("(no commands in this session)\n");
    }
    Ok(out)
}

/// Interactive drill: pick a session where `cmd` ran; preview shows its timeline.
/// Ctrl-e opens the highlighted session's timeline in $EDITOR. Read-only.
fn context_drill(conn: &rusqlite::Connection, cmd: &str) -> Result<()> {
    use std::io::Write;

    let sessions = db::context_sessions(conn, cmd)?;
    if sessions.is_empty() {
        eprintln!("no recorded sessions for: {cmd}");
        return Ok(());
    }

    let self_exe = std::env::current_exe().unwrap_or_else(|_| "hindsight".into());
    let exe = self_exe.to_string_lossy().to_string();

    let mut child = std::process::Command::new("fzf")
        .args([
            "--delimiter=\t",
            "--with-nth=2..",
            "--layout=reverse",
            "--preview-window=right,60%,wrap",
        ])
        .arg(format!("--header=sessions where: {cmd}   up/down: switch   ctrl-e: open in $EDITOR"))
        .arg(format!(
            "--preview={exe} context timeline --session {{1}} -- \"$HINDSIGHT_CONTEXT_CMD\""
        ))
        .arg(format!(
            "--bind=ctrl-e:execute({exe} context edit --session {{1}})"
        ))
        .env("HINDSIGHT_CONTEXT_CMD", cmd)
        .stdin(std::process::Stdio::piped())
        .spawn()?;

    if let Some(stdin) = child.stdin.as_mut() {
        for s in &sessions {
            writeln!(stdin, "{}\t{}\t{}", s.session, s.label, s.count)?;
        }
    }
    // Read-only exploration: ignore the selection, just wait for fzf to close.
    let _ = child.wait()?;
    Ok(())
}

/// View mode persisted in the picker state file.
const MODE_HISTORY: &str = "history";
const MODE_FAVORITES: &str = "favorites";

/// Drive one refresh of the fzf picker.
///
/// Reads (and possibly mutates) the mode state file, applies any star toggle,
/// then prints the current view as tab-delimited `<marker>\t<cmd>` rows, where
/// marker is "★" for favorites, "✎" for a note (concatenated if both).
fn picker(
    conn: &rusqlite::Connection,
    state_path: &str,
    toggle: bool,
    star_toggle: bool,
    cmd: &str,
) -> Result<()> {
    let path = std::path::Path::new(state_path);
    let mut mode = match std::fs::read_to_string(path) {
        Ok(s) if s.trim() == MODE_FAVORITES => MODE_FAVORITES,
        _ => MODE_HISTORY,
    };

    if toggle {
        mode = if mode == MODE_HISTORY {
            MODE_FAVORITES
        } else {
            MODE_HISTORY
        };
        std::fs::write(path, mode)?;
    }

    if star_toggle && !cmd.is_empty() {
        db::fav_toggle(conn, cmd)?;
    }

    if mode == MODE_FAVORITES {
        for (cmd, has_note) in db::fav_rows(conn)? {
            let marker = marker(true, has_note);
            println!("{marker}\t{cmd}");
        }
    } else {
        for (cmd, is_fav, has_note) in db::list_rows(conn, None, None, None)? {
            let marker = marker(is_fav, has_note);
            println!("{marker}\t{cmd}");
        }
    }
    Ok(())
}

/// Build the marker column: ★ for favorite, ✎ for note (both if applicable).
fn marker(is_fav: bool, has_note: bool) -> String {
    let mut m = String::new();
    if is_fav {
        m.push('★');
    }
    if has_note {
        m.push('✎');
    }
    m
}
