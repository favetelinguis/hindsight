use anyhow::{Context, Result};
use rusqlite::Connection;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// Resolve the data directory: `$_HINDSIGHT_DATA_DIR` if set, else the platform
/// data dir under `hindsight/` (e.g. ~/Library/Application Support/hindsight on macOS,
/// $XDG_DATA_HOME/hindsight or ~/.local/share/hindsight on Linux).
fn data_dir() -> Result<PathBuf> {
    if let Ok(dir) = std::env::var("_HINDSIGHT_DATA_DIR") {
        if !dir.is_empty() {
            return Ok(PathBuf::from(dir));
        }
    }
    let base = dirs::data_dir().context("could not determine platform data directory")?;
    Ok(base.join("hindsight"))
}

/// Open the database, creating the directory and schema if needed.
pub fn open() -> Result<Connection> {
    let dir = data_dir()?;
    std::fs::create_dir_all(&dir)
        .with_context(|| format!("creating data dir {}", dir.display()))?;
    let path = dir.join("history.db");
    let conn = Connection::open(&path)
        .with_context(|| format!("opening database {}", path.display()))?;
    migrate(&conn)?;
    Ok(conn)
}

fn migrate(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "
        CREATE TABLE IF NOT EXISTS commands (
            id         INTEGER PRIMARY KEY,
            cmd        TEXT NOT NULL,
            cwd        TEXT NOT NULL,
            exit_code  INTEGER,
            start_ts   INTEGER NOT NULL,
            end_ts     INTEGER,
            session    TEXT NOT NULL
        );
        CREATE INDEX IF NOT EXISTS idx_cmd_prefix ON commands(cmd);
        CREATE INDEX IF NOT EXISTS idx_cmd_cwd     ON commands(cwd);
        CREATE INDEX IF NOT EXISTS idx_cmd_start   ON commands(start_ts);
        CREATE TABLE IF NOT EXISTS pending (
            session   TEXT PRIMARY KEY,
            cmd       TEXT NOT NULL,
            cwd       TEXT NOT NULL,
            start_ts  INTEGER NOT NULL
        );
        CREATE TABLE IF NOT EXISTS favorites (
            cmd         TEXT PRIMARY KEY,
            created_ts  INTEGER NOT NULL
        );
        CREATE TABLE IF NOT EXISTS notes (
            cmd  TEXT PRIMARY KEY,
            note TEXT NOT NULL
        );
        CREATE TABLE IF NOT EXISTS deleted (
            cmd         TEXT PRIMARY KEY,
            deleted_ts  INTEGER NOT NULL
        );
        -- Fresh DBs get this 3-column shape. Databases created before session
        -- ids went terminal-agnostic keep leftover nullable `term`/`pane`
        -- columns; they go unwritten and unread (intentional schema drift, no
        -- ALTER TABLE migration needed).
        CREATE TABLE IF NOT EXISTS sessions (
            session     TEXT PRIMARY KEY,
            started_ts  INTEGER NOT NULL,
            init_cwd    TEXT NOT NULL
        );
        ",
    )?;
    Ok(())
}

/// preexec: record a pending command for this session.
///
/// `ignore` is the compiled ignore-list; a command matching any pattern is
/// silently not recorded.
pub fn start(
    conn: &Connection,
    session: &str,
    cwd: &str,
    cmd: &str,
    ignore: &[regex::Regex],
) -> Result<()> {
    let cmd = cmd.trim();
    if cmd.is_empty() {
        return Ok(());
    }
    if crate::config::is_ignored(cmd, ignore) {
        return Ok(());
    }
    // Record session metadata on first sighting (first recorded command wins).
    // Naming only `session, started_ts, init_cwd` keeps this valid against both
    // the current schema and older DBs that still carry nullable term/pane.
    conn.execute(
        "INSERT OR IGNORE INTO sessions (session, started_ts, init_cwd)
         VALUES (?1, ?2, ?3)",
        rusqlite::params![session, now(), cwd],
    )?;
    conn.execute(
        "INSERT INTO pending (session, cmd, cwd, start_ts) VALUES (?1, ?2, ?3, ?4)
         ON CONFLICT(session) DO UPDATE SET cmd = ?2, cwd = ?3, start_ts = ?4",
        rusqlite::params![session, cmd, cwd, now()],
    )?;
    Ok(())
}

/// precmd: finalize the pending command for this session with its exit code.
pub fn end(conn: &Connection, session: &str, exit_code: i64) -> Result<()> {
    let pending: Option<(String, String, i64)> = conn
        .query_row(
            "SELECT cmd, cwd, start_ts FROM pending WHERE session = ?1",
            [session],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .ok();
    let Some((cmd, cwd, start_ts)) = pending else {
        return Ok(());
    };
    conn.execute(
        "INSERT INTO commands (cmd, cwd, exit_code, start_ts, end_ts, session)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        rusqlite::params![cmd, cwd, exit_code, start_ts, now(), session],
    )?;
    conn.execute("DELETE FROM pending WHERE session = ?1", [session])?;
    Ok(())
}

/// `query --list`: newest-first, deduped command lines. Optional cwd/exit filters.
pub fn list(
    conn: &Connection,
    cwd: Option<&str>,
    exit: Option<i64>,
    limit: Option<i64>,
) -> Result<Vec<String>> {
    Ok(list_rows(conn, cwd, exit, limit)?
        .into_iter()
        .map(|(cmd, _, _)| cmd)
        .collect())
}

/// Like `list`, but each row carries whether the command is a favorite and
/// whether it has a note. Powers the history view of the fzf picker (★/✎ markers).
pub fn list_rows(
    conn: &Connection,
    cwd: Option<&str>,
    exit: Option<i64>,
    limit: Option<i64>,
) -> Result<Vec<(String, bool, bool)>> {
    // Group by command text, keeping the most recent occurrence for ordering.
    // MAX(id) is the recency tiebreaker: start_ts is second-granularity, so
    // commands run in the same second would otherwise order arbitrarily.
    let mut sql = String::from(
        "SELECT c.cmd, MAX(c.id) AS mid,
                MAX(CASE WHEN f.cmd IS NOT NULL THEN 1 ELSE 0 END) AS is_fav,
                MAX(CASE WHEN n.note IS NOT NULL AND n.note <> '' THEN 1 ELSE 0 END) AS has_note
         FROM commands c
         LEFT JOIN favorites f ON f.cmd = c.cmd
         LEFT JOIN notes n ON n.cmd = c.cmd
         WHERE c.cmd NOT IN (SELECT cmd FROM deleted)",
    );
    let mut params: Vec<Box<dyn rusqlite::ToSql>> = Vec::new();
    if let Some(c) = cwd {
        sql.push_str(" AND c.cwd = ?");
        params.push(Box::new(c.to_string()));
    }
    if let Some(e) = exit {
        sql.push_str(" AND c.exit_code = ?");
        params.push(Box::new(e));
    }
    sql.push_str(" GROUP BY c.cmd ORDER BY mid DESC");
    if let Some(l) = limit {
        sql.push_str(" LIMIT ?");
        params.push(Box::new(l));
    }
    let mut stmt = conn.prepare(&sql)?;
    let param_refs: Vec<&dyn rusqlite::ToSql> = params.iter().map(|p| p.as_ref()).collect();
    let rows = stmt.query_map(param_refs.as_slice(), |r| {
        Ok((
            r.get::<_, String>(0)?,
            r.get::<_, i64>(2)? != 0,
            r.get::<_, i64>(3)? != 0,
        ))
    })?;
    let mut out = Vec::new();
    for row in rows {
        out.push(row?);
    }
    Ok(out)
}

/// Add a command to favorites (idempotent). Trims; ignores empty.
pub fn fav_add(conn: &Connection, cmd: &str) -> Result<()> {
    let cmd = cmd.trim();
    if cmd.is_empty() {
        return Ok(());
    }
    conn.execute(
        "INSERT OR IGNORE INTO favorites (cmd, created_ts) VALUES (?1, ?2)",
        rusqlite::params![cmd, now()],
    )?;
    Ok(())
}

/// Remove a command from favorites.
pub fn fav_remove(conn: &Connection, cmd: &str) -> Result<()> {
    conn.execute("DELETE FROM favorites WHERE cmd = ?1", [cmd.trim()])?;
    Ok(())
}

/// Toggle a command's favorite state. Returns the new state (true = now starred).
pub fn fav_toggle(conn: &Connection, cmd: &str) -> Result<bool> {
    let cmd = cmd.trim();
    if cmd.is_empty() {
        return Ok(false);
    }
    let exists: bool = conn
        .query_row(
            "SELECT 1 FROM favorites WHERE cmd = ?1",
            [cmd],
            |_| Ok(()),
        )
        .is_ok();
    if exists {
        fav_remove(conn, cmd)?;
        Ok(false)
    } else {
        fav_add(conn, cmd)?;
        Ok(true)
    }
}

/// List favorites, newest-first.
pub fn fav_list(conn: &Connection) -> Result<Vec<String>> {
    Ok(fav_rows(conn)?.into_iter().map(|(cmd, _)| cmd).collect())
}

/// List favorites, newest-first, each with whether it has a note (for the ✎ marker).
pub fn fav_rows(conn: &Connection) -> Result<Vec<(String, bool)>> {
    let mut stmt = conn.prepare(
        "SELECT f.cmd,
                CASE WHEN n.note IS NOT NULL AND n.note <> '' THEN 1 ELSE 0 END AS has_note
         FROM favorites f
         LEFT JOIN notes n ON n.cmd = f.cmd
         WHERE f.cmd NOT IN (SELECT cmd FROM deleted)
         ORDER BY f.created_ts DESC, f.cmd ASC",
    )?;
    let rows = stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)? != 0)))?;
    let mut out = Vec::new();
    for row in rows {
        out.push(row?);
    }
    Ok(out)
}

/// Set (or clear) a command's note. An empty note deletes the row. Trims cmd.
pub fn note_set(conn: &Connection, cmd: &str, note: &str) -> Result<()> {
    let cmd = cmd.trim();
    if cmd.is_empty() {
        return Ok(());
    }
    if note.trim().is_empty() {
        conn.execute("DELETE FROM notes WHERE cmd = ?1", [cmd])?;
    } else {
        conn.execute(
            "INSERT INTO notes (cmd, note) VALUES (?1, ?2)
             ON CONFLICT(cmd) DO UPDATE SET note = ?2",
            rusqlite::params![cmd, note],
        )?;
    }
    Ok(())
}

/// Get a command's note, or "" if none.
pub fn note_get(conn: &Connection, cmd: &str) -> Result<String> {
    Ok(conn
        .query_row(
            "SELECT note FROM notes WHERE cmd = ?1",
            [cmd.trim()],
            |r| r.get::<_, String>(0),
        )
        .unwrap_or_default())
}

/// Soft-delete a command: mark it hidden from all user-facing views. NOTHING is
/// physically removed — the command's history/favorite/note rows stay in the DB
/// and can be inspected via `deleted_*` or brought back with `restore`.
pub fn soft_delete(conn: &Connection, cmd: &str) -> Result<()> {
    let cmd = cmd.trim();
    if cmd.is_empty() {
        return Ok(());
    }
    conn.execute(
        "INSERT OR REPLACE INTO deleted (cmd, deleted_ts) VALUES (?1, ?2)",
        rusqlite::params![cmd, now()],
    )?;
    Ok(())
}

/// Un-delete a soft-deleted command so it reappears in normal views.
pub fn restore(conn: &Connection, cmd: &str) -> Result<()> {
    conn.execute("DELETE FROM deleted WHERE cmd = ?1", [cmd.trim()])?;
    Ok(())
}

/// True if the command is currently soft-deleted.
pub fn is_deleted(conn: &Connection, cmd: &str) -> Result<bool> {
    Ok(conn
        .query_row("SELECT 1 FROM deleted WHERE cmd = ?1", [cmd.trim()], |_| Ok(()))
        .is_ok())
}

// ============================================================================
// Metadata / aggregation layer (read-only) — powers `inspect` and `stats`.
// ============================================================================

/// Full metadata for a single command.
pub struct CommandStats {
    pub run_count: i64,
    /// (exit_code, count); `None` code = command never finalized (no exit recorded).
    pub exit_codes: Vec<(Option<i64>, i64)>,
    /// (cwd, count), most-used directory first.
    pub directories: Vec<(String, i64)>,
    pub note: String,
    pub is_favorite: bool,
    pub first_run: Option<i64>,
    pub last_run: Option<i64>,
}

/// Gather all metadata for one command. Returns `None` for a command that has
/// never been recorded and is neither favorited nor noted (i.e. unknown).
///
/// Active-only: a soft-deleted command is treated as unknown (returns `None`).
/// For inspecting soft-deleted commands, use `inspect_any`.
pub fn inspect(conn: &Connection, cmd: &str) -> Result<Option<CommandStats>> {
    if is_deleted(conn, cmd)? {
        return Ok(None);
    }
    inspect_any(conn, cmd)
}

/// Like `inspect`, but does NOT hide soft-deleted commands — used by the
/// agent-only `deleted inspect` command.
pub fn inspect_any(conn: &Connection, cmd: &str) -> Result<Option<CommandStats>> {
    let cmd = cmd.trim();
    if cmd.is_empty() {
        return Ok(None);
    }

    let run_count: i64 =
        conn.query_row("SELECT COUNT(*) FROM commands WHERE cmd = ?1", [cmd], |r| {
            r.get(0)
        })?;
    let is_favorite: bool = conn
        .query_row("SELECT 1 FROM favorites WHERE cmd = ?1", [cmd], |_| Ok(()))
        .is_ok();
    let note = note_get(conn, cmd)?;

    if run_count == 0 && !is_favorite && note.is_empty() {
        return Ok(None);
    }

    let mut stmt = conn.prepare(
        "SELECT exit_code, COUNT(*) FROM commands WHERE cmd = ?1
         GROUP BY exit_code ORDER BY exit_code",
    )?;
    let exit_codes = stmt
        .query_map([cmd], |r| Ok((r.get::<_, Option<i64>>(0)?, r.get::<_, i64>(1)?)))?
        .collect::<std::result::Result<Vec<_>, _>>()?;

    let mut stmt = conn.prepare(
        "SELECT cwd, COUNT(*) AS n FROM commands WHERE cmd = ?1
         GROUP BY cwd ORDER BY n DESC, cwd ASC",
    )?;
    let directories = stmt
        .query_map([cmd], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)))?
        .collect::<std::result::Result<Vec<_>, _>>()?;

    let (first_run, last_run): (Option<i64>, Option<i64>) = conn.query_row(
        "SELECT MIN(start_ts), MAX(start_ts) FROM commands WHERE cmd = ?1",
        [cmd],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )?;

    Ok(Some(CommandStats {
        run_count,
        exit_codes,
        directories,
        note,
        is_favorite,
        first_run,
        last_run,
    }))
}

/// Most-run commands: (cmd, run_count), descending.
pub fn stats_top_commands(conn: &Connection, limit: i64) -> Result<Vec<(String, i64)>> {
    let mut stmt = conn.prepare(
        "SELECT cmd, COUNT(*) AS n FROM commands
         WHERE cmd NOT IN (SELECT cmd FROM deleted)
         GROUP BY cmd ORDER BY n DESC, cmd ASC LIMIT ?1",
    )?;
    let out = stmt
        .query_map([limit], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)))?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    Ok(out)
}

/// Commands with the most failures: (cmd, failures, total_runs). Only those with
/// at least one non-zero (and non-null) exit code, ordered by failures desc.
pub fn stats_error_prone(conn: &Connection, limit: i64) -> Result<Vec<(String, i64, i64)>> {
    let mut stmt = conn.prepare(
        "SELECT cmd,
                SUM(CASE WHEN exit_code <> 0 AND exit_code IS NOT NULL THEN 1 ELSE 0 END) AS fails,
                COUNT(*) AS total
         FROM commands
         WHERE cmd NOT IN (SELECT cmd FROM deleted)
         GROUP BY cmd
         HAVING fails > 0
         ORDER BY fails DESC, total DESC, cmd ASC
         LIMIT ?1",
    )?;
    let out = stmt
        .query_map([limit], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?, r.get::<_, i64>(2)?))
        })?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    Ok(out)
}

/// Most-used directories: (cwd, count), descending. Ignores empty cwd (imports).
pub fn stats_top_dirs(conn: &Connection, limit: i64) -> Result<Vec<(String, i64)>> {
    let mut stmt = conn.prepare(
        "SELECT cwd, COUNT(*) AS n FROM commands
         WHERE cwd <> ''
           AND cmd NOT IN (SELECT cmd FROM deleted)
         GROUP BY cwd ORDER BY n DESC, cwd ASC LIMIT ?1",
    )?;
    let out = stmt
        .query_map([limit], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)))?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    Ok(out)
}

/// Global scalar totals across the whole database.
pub struct Totals {
    pub distinct_commands: i64,
    pub total_runs: i64,
    pub favorites: i64,
    pub notes: i64,
}

pub fn stats_totals(conn: &Connection) -> Result<Totals> {
    // All counts exclude soft-deleted commands for consistency with the views.
    Ok(Totals {
        distinct_commands: conn.query_row(
            "SELECT COUNT(DISTINCT cmd) FROM commands WHERE cmd NOT IN (SELECT cmd FROM deleted)",
            [],
            |r| r.get(0),
        )?,
        total_runs: conn.query_row(
            "SELECT COUNT(*) FROM commands WHERE cmd NOT IN (SELECT cmd FROM deleted)",
            [],
            |r| r.get(0),
        )?,
        favorites: conn.query_row(
            "SELECT COUNT(*) FROM favorites WHERE cmd NOT IN (SELECT cmd FROM deleted)",
            [],
            |r| r.get(0),
        )?,
        notes: conn.query_row(
            "SELECT COUNT(*) FROM notes WHERE cmd NOT IN (SELECT cmd FROM deleted)",
            [],
            |r| r.get(0),
        )?,
    })
}

/// `prune ignore`: commands (excluding already-deleted) matching the ignore list,
/// as (cmd, run_count). When `apply`, each match is soft-deleted. Returns the
/// matched set either way (for the report).
pub fn prune_ignored(
    conn: &Connection,
    ignore: &[regex::Regex],
    apply: bool,
) -> Result<Vec<(String, i64)>> {
    let mut stmt = conn.prepare(
        "SELECT cmd, COUNT(*) AS n FROM commands
         WHERE cmd NOT IN (SELECT cmd FROM deleted)
         GROUP BY cmd ORDER BY n DESC, cmd ASC",
    )?;
    let all = stmt
        .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)))?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    let matched: Vec<(String, i64)> = all
        .into_iter()
        .filter(|(cmd, _)| crate::config::is_ignored(cmd, ignore))
        .collect();
    if apply {
        for (cmd, _) in &matched {
            soft_delete(conn, cmd)?;
        }
    }
    Ok(matched)
}

/// List soft-deleted commands: (cmd, deleted_ts, run_count), newest deletion first.
pub fn deleted_list(conn: &Connection) -> Result<Vec<(String, i64, i64)>> {
    let mut stmt = conn.prepare(
        "SELECT d.cmd, d.deleted_ts,
                (SELECT COUNT(*) FROM commands c WHERE c.cmd = d.cmd) AS run_count
         FROM deleted d
         ORDER BY d.deleted_ts DESC, d.cmd ASC",
    )?;
    let out = stmt
        .query_map([], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?, r.get::<_, i64>(2)?))
        })?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    Ok(out)
}

/// The deletion timestamp for a soft-deleted command, if any.
pub fn deleted_ts(conn: &Connection, cmd: &str) -> Result<Option<i64>> {
    Ok(conn
        .query_row(
            "SELECT deleted_ts FROM deleted WHERE cmd = ?1",
            [cmd.trim()],
            |r| r.get::<_, i64>(0),
        )
        .ok())
}

/// cwd-aware prefix search for the Up-arrow widget.
/// Returns commands matching `prefix`, ranked cwd-matches first, newest-first,
/// deduped. `offset` selects the Nth match (for repeated Up presses).
pub fn search(conn: &Connection, cwd: &str, prefix: &str, offset: i64) -> Result<Option<String>> {
    let like = format!("{}%", escape_like(prefix));
    // MAX(id) breaks recency ties within the same second (see `list`).
    let mut stmt = conn.prepare(
        "SELECT cmd FROM (
             SELECT cmd,
                    MAX(id) AS mid,
                    MAX(CASE WHEN cwd = ?1 THEN 1 ELSE 0 END) AS here
             FROM commands
             WHERE cmd LIKE ?2 ESCAPE '\\'
               AND cmd NOT IN (SELECT cmd FROM deleted)
             GROUP BY cmd
         )
         ORDER BY here DESC, mid DESC
         LIMIT 1 OFFSET ?3",
    )?;
    let result = stmt
        .query_row(rusqlite::params![cwd, like, offset.max(0)], |r| {
            r.get::<_, String>(0)
        })
        .ok();
    Ok(result)
}

/// Escape `%` and `_` so a user-typed prefix is matched literally in LIKE.
fn escape_like(s: &str) -> String {
    s.replace('\\', "\\\\").replace('%', "\\%").replace('_', "\\_")
}

/// Import commands from an existing zsh history file (best-effort).
pub fn import_zsh(conn: &Connection, path: &std::path::Path) -> Result<usize> {
    let content = std::fs::read_to_string(path)
        .with_context(|| format!("reading {}", path.display()))?;
    let tx = conn.unchecked_transaction()?;
    let mut count = 0;
    for line in content.lines() {
        // zsh extended history format: ": <start>:<elapsed>;<command>"
        let cmd = if let Some(rest) = line.strip_prefix(':') {
            rest.splitn(2, ';').nth(1).unwrap_or("").trim()
        } else {
            line.trim()
        };
        if cmd.is_empty() {
            continue;
        }
        tx.execute(
            "INSERT INTO commands (cmd, cwd, exit_code, start_ts, end_ts, session)
             VALUES (?1, '', NULL, ?2, NULL, 'import')",
            rusqlite::params![cmd, now()],
        )?;
        count += 1;
    }
    tx.commit()?;
    Ok(count)
}

// ============================================================================
// Usage context — which sessions a command ran in, and their timelines.
// ============================================================================

/// One session in which a command appeared.
pub struct SessionUsage {
    pub session: String,
    pub label: String,
    pub count: i64,
    /// Most-recent run in this session (drives ordering; not otherwise read).
    #[allow(dead_code)]
    pub last_ts: i64,
}

/// One command in a session's timeline.
pub struct TimelineRow {
    pub cmd: String,
    pub exit_code: Option<i64>,
    pub start_ts: i64,
}

/// Abbreviate a home-prefixed path with `~`.
fn abbreviate_home(path: &str) -> String {
    if let Some(home) = dirs::home_dir() {
        if let Some(home) = home.to_str() {
            if path == home {
                return "~".to_string();
            }
            if let Some(rest) = path.strip_prefix(&format!("{home}/")) {
                return format!("~/{rest}");
            }
        }
    }
    path.to_string()
}

/// Human label for a session: its initial working directory (home-abbreviated),
/// falling back to the raw session id when no cwd is recorded.
fn session_label(session: &str, init_cwd: &str) -> String {
    if init_cwd.is_empty() {
        session.to_string()
    } else {
        abbreviate_home(init_cwd)
    }
}

/// Distinct sessions in which `cmd` ran (active-only), each with occurrence
/// count and most-recent run, newest first.
pub fn context_sessions(conn: &Connection, cmd: &str) -> Result<Vec<SessionUsage>> {
    let cmd = cmd.trim();
    let mut stmt = conn.prepare(
        "SELECT c.session, COUNT(*) AS n, MAX(c.start_ts) AS last_ts,
                COALESCE(s.init_cwd, '') AS init_cwd
         FROM commands c
         LEFT JOIN sessions s ON s.session = c.session
         WHERE c.cmd = ?1
           AND c.cmd NOT IN (SELECT cmd FROM deleted)
         GROUP BY c.session
         ORDER BY last_ts DESC",
    )?;
    let rows = stmt
        .query_map([cmd], |r| {
            let session: String = r.get(0)?;
            let count: i64 = r.get(1)?;
            let last_ts: i64 = r.get(2)?;
            let init_cwd: String = r.get(3)?;
            let label = session_label(&session, &init_cwd);
            Ok(SessionUsage { session, label, count, last_ts })
        })?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    Ok(rows)
}

/// Every command in a session, ordered chronologically (active-only).
pub fn session_timeline(conn: &Connection, session: &str) -> Result<Vec<TimelineRow>> {
    let mut stmt = conn.prepare(
        "SELECT cmd, exit_code, start_ts FROM commands
         WHERE session = ?1
           AND cmd NOT IN (SELECT cmd FROM deleted)
         ORDER BY id ASC",
    )?;
    let rows = stmt
        .query_map([session], |r| {
            Ok(TimelineRow {
                cmd: r.get(0)?,
                exit_code: r.get(1)?,
                start_ts: r.get(2)?,
            })
        })?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    Ok(rows)
}
