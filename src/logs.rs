//! The record of heavy commands: every run once it is over, kept 30 days, and `heavy-cmd logs`.

use crate::format::duration;
use chrono::{DateTime, Local};
use rusqlite::{Connection, OpenFlags, params};
use std::path::Path;
use std::time::Duration;

/// How long a run is kept.
const KEEP_MS: i64 = 30 * 86_400_000;

/// How many runs `heavy-cmd logs` lists.
pub const LIMIT: u32 = 50;

pub const RAN: &str = "ran";
pub const GAVE_UP: &str = "gave up";
pub const STOPPED: &str = "stopped while waiting";

pub const RUNS: &str = "
CREATE TABLE IF NOT EXISTS runs (
  id INTEGER PRIMARY KEY AUTOINCREMENT, command TEXT NOT NULL, cwd TEXT NOT NULL, queued_at INTEGER NOT NULL,
  started_at INTEGER, ended_at INTEGER NOT NULL, exit INTEGER, outcome TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS runs_ended_at ON runs (ended_at);
";

/// One run of a heavy command, its times in ms.
#[derive(Debug, Clone, PartialEq)]
pub struct Run {
    /// As it was given: a shell line, or the argv joined by spaces.
    pub command: String,
    pub cwd: String,
    pub queued_at: i64,
    /// None when it never started.
    pub started_at: Option<i64>,
    pub ended_at: i64,
    /// The command's exit when it ran, heavy-cmd's when it did not.
    pub exit: Option<i64>,
    pub outcome: &'static str,
}

/// Keeps `run`, and forgets the runs over 30 days old.
pub fn record(db: &Connection, run: &Run, now: i64) -> rusqlite::Result<()> {
    db.execute(
        "INSERT INTO runs (command, cwd, queued_at, started_at, ended_at, exit, outcome) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        params![run.command, run.cwd, run.queued_at, run.started_at, run.ended_at, run.exit, run.outcome],
    )?;
    db.execute("DELETE FROM runs WHERE ended_at < ?1", [now - KEEP_MS])?;
    Ok(())
}

/// The last `limit` runs of the record in `dir`, the last ended first, read with heavy.db opened
/// read-only; none before the record.
pub fn recent(dir: &Path, limit: u32) -> rusqlite::Result<Vec<Run>> {
    let file = dir.join("heavy.db");
    if !file.exists() {
        return Ok(Vec::new());
    }
    let db = Connection::open_with_flags(file, OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX)?;
    db.busy_timeout(Duration::from_secs(5))?;
    let exists: bool =
        db.query_row("SELECT EXISTS (SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = 'runs')", [], |row| row.get(0))?;
    if !exists {
        return Ok(Vec::new());
    }
    let mut statement = db.prepare(
        "SELECT command, cwd, queued_at, started_at, ended_at, exit, outcome FROM runs ORDER BY ended_at DESC, id DESC LIMIT ?1",
    )?;
    let runs = statement.query_map([limit], |row| {
        let outcome: String = row.get(6)?;
        Ok(Run {
            command: row.get(0)?,
            cwd: row.get(1)?,
            queued_at: row.get(2)?,
            started_at: row.get(3)?,
            ended_at: row.get(4)?,
            exit: row.get(5)?,
            outcome: [RAN, GAVE_UP, STOPPED].into_iter().find(|known| *known == outcome).unwrap_or(GAVE_UP),
        })
    })?;
    runs.collect()
}

/// The local time of `at`, `14:02:11`, its date too on another day than `now`'s: `Nov 12 14:02:11`.
pub fn clock(at: i64, now: i64) -> String {
    let local = |ms| DateTime::from_timestamp_millis(ms).unwrap_or_default().with_timezone(&Local);
    let date = local(at);
    let format = if date.date_naive() == local(now).date_naive() { "%H:%M:%S" } else { "%b %-d %H:%M:%S" };
    date.format(format).to_string()
}

/// `~/workspace/api  npm run check  exit 0  waited 12s  ran 3m 4s`, or `... gave up after 15m (75)`: a
/// run without when it ended; `home` is shown as `~`.
pub fn line(run: &Run, home: Option<&Path>) -> String {
    let cwd = home.and_then(|home| Path::new(&run.cwd).strip_prefix(home).ok()).map_or_else(
        || run.cwd.clone(),
        |rest| if rest.as_os_str().is_empty() { "~".into() } else { format!("~{}{}", std::path::MAIN_SEPARATOR, rest.display()) },
    );
    let exit = run.exit.map_or_else(|| "?".to_string(), |exit| exit.to_string());
    let how = match run.started_at {
        Some(started) => format!("exit {exit}  waited {}  ran {}", duration(started - run.queued_at), duration(run.ended_at - started)),
        None => format!("{} after {} ({exit})", run.outcome, duration(run.ended_at - run.queued_at)),
    };
    format!("{cwd}  {}  {how}", run.command)
}
