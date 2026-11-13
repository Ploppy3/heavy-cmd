use crate::format::{iso, parse_time};
use rusqlite::{Connection, Row, params, params_from_iter, types::Value as Sql};
use serde_json::{Map, Value};

/// How a heavy command ended: it ran, or its wait ended before it could.
pub const RAN: &str = "ran";
pub const GAVE_UP: &str = "gave up";
pub const STOPPED: &str = "stopped while waiting";

/// A heavy command once it is over.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct HeavyRun {
    pub id: i64,
    pub project: String,
    pub label: String,
    /// One argument is its shell line, several the program and its arguments; None for work that is not a command.
    pub command: Option<Vec<String>>,
    pub cwd: String,
    pub host: String,
    pub pid: i64,
    /// The agent that ran it: `--agent`, else `HEAVY_CMD_AGENT`; None when neither is set.
    pub agent: Option<String>,
    /// Its id in the queue, the `HEAVY_CMD_ID` its own commands see; None when it never started there.
    pub heavy_id: Option<i64>,
    /// The heavy command it ran inside, by that one's `heavy_id`: its `HEAVY_CMD_ID`.
    pub inside: Option<i64>,
    pub queued_at: String,
    /// None when it never started.
    pub started_at: Option<String>,
    pub ended_at: String,
    /// The command's exit code, `128 + signal` for a signal; None for a wait stopped otherwise than by a signal.
    pub exit: Option<i64>,
    pub outcome: String,
}

impl HeavyRun {
    /// Every field, in the record's order.
    pub fn to_json(&self) -> Value {
        let mut run = Map::new();
        run.insert("id".into(), self.id.into());
        run.insert("project".into(), self.project.clone().into());
        run.insert("label".into(), self.label.clone().into());
        run.insert("command".into(), self.command.clone().into());
        run.insert("cwd".into(), self.cwd.clone().into());
        run.insert("host".into(), self.host.clone().into());
        run.insert("pid".into(), self.pid.into());
        run.insert("agent".into(), self.agent.clone().into());
        run.insert("heavy_id".into(), self.heavy_id.into());
        run.insert("inside".into(), self.inside.into());
        run.insert("queued_at".into(), self.queued_at.clone().into());
        run.insert("started_at".into(), self.started_at.clone().into());
        run.insert("ended_at".into(), self.ended_at.clone().into());
        run.insert("exit".into(), self.exit.into());
        run.insert("outcome".into(), self.outcome.clone().into());
        Value::Object(run)
    }

    fn of(row: &Row) -> rusqlite::Result<HeavyRun> {
        let command: Option<String> = row.get("command")?;
        Ok(HeavyRun {
            id: row.get("id")?,
            project: row.get("project")?,
            label: row.get("label")?,
            command: command.and_then(|text| serde_json::from_str(&text).ok()),
            cwd: row.get("cwd")?,
            host: row.get("host")?,
            pid: row.get("pid")?,
            agent: row.get("agent")?,
            heavy_id: row.get("heavy_id")?,
            inside: row.get("inside")?,
            queued_at: row.get("queued_at")?,
            started_at: row.get("started_at")?,
            ended_at: row.get("ended_at")?,
            exit: row.get("exit")?,
            outcome: row.get("outcome")?,
        })
    }
}

pub const RUNS: &str = "
CREATE TABLE IF NOT EXISTS runs (
  id INTEGER PRIMARY KEY AUTOINCREMENT, project TEXT NOT NULL, label TEXT NOT NULL, command TEXT, cwd TEXT NOT NULL,
  host TEXT NOT NULL, pid INTEGER NOT NULL, agent TEXT, heavy_id INTEGER, inside INTEGER,
  queued_at TEXT NOT NULL, started_at TEXT, ended_at TEXT NOT NULL, exit INTEGER, outcome TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS runs_ended ON runs (ended_at);
";

/// How long a run is kept once it is over.
const KEEP_MS: i64 = 30 * 86_400_000;

/// Records `run` (its id aside), and forgets the runs that ended over 30 days before `now`.
pub fn record_run(db: &Connection, run: &HeavyRun, now: i64) -> rusqlite::Result<()> {
    db.execute("DELETE FROM runs WHERE ended_at < ?1", [iso(now - KEEP_MS)])?;
    let command = run.command.as_ref().map(|argv| serde_json::to_string(argv).unwrap_or_default());
    db.execute(
        "INSERT INTO runs (project, label, command, cwd, host, pid, agent, heavy_id, inside, queued_at, started_at, ended_at, exit, outcome)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)",
        params![
            run.project,
            run.label,
            command,
            run.cwd,
            run.host,
            run.pid,
            run.agent,
            run.heavy_id,
            run.inside,
            run.queued_at,
            run.started_at,
            run.ended_at,
            run.exit,
            run.outcome
        ],
    )?;
    Ok(())
}

/// Which runs to read: one project's or the machine's, those that ended at `since` or later, the newest `limit`.
#[derive(Debug, Default, Clone)]
pub struct RunFilter {
    pub project: Option<String>,
    pub limit: u64,
    pub since: Option<String>,
}

/// The runs `filter` keeps, the last ended first.
pub fn runs_of(db: &Connection, filter: &RunFilter) -> rusqlite::Result<Vec<HeavyRun>> {
    let mut clauses = Vec::new();
    let mut values = Vec::new();
    if let Some(project) = &filter.project {
        clauses.push("project = ?");
        values.push(Sql::Text(project.clone()));
    }
    if let Some(since) = &filter.since {
        clauses.push("ended_at >= ?");
        values.push(Sql::Text(since.clone()));
    }
    values.push(Sql::Integer(filter.limit as i64));
    let clause = if clauses.is_empty() { String::new() } else { format!("WHERE {}", clauses.join(" AND ")) };
    let mut statement = db.prepare(&format!("SELECT * FROM runs {clause} ORDER BY ended_at DESC, id DESC LIMIT ?"))?;
    statement.query_map(params_from_iter(values), HeavyRun::of)?.collect()
}

/// `90s`, `15m`, `1h30m`, `2d` in ms; None for anything else.
fn duration_ms(text: &str) -> Option<i64> {
    let mut total: i64 = 0;
    let mut digits = String::new();
    for c in text.chars() {
        if c.is_ascii_digit() {
            digits.push(c);
            continue;
        }
        let unit = match c {
            's' => 1_000,
            'm' => 60_000,
            'h' => 3_600_000,
            'd' => 86_400_000,
            _ => return None,
        };
        let n: i64 = digits.parse().ok()?;
        total = total.checked_add(n.checked_mul(unit)?)?;
        digits.clear();
    }
    (digits.is_empty() && !text.is_empty()).then_some(total)
}

/// `raw`: an ISO time, or a duration back from `now` such as `90s`, `15m`, `1h30m` or `2d`, as an ISO
/// time; else why not, naming `name`.
pub fn since_of(raw: &str, now: i64, name: &str) -> Result<String, String> {
    let text = raw.trim();
    let dated = text.len() >= 10
        && text.as_bytes()[..10].iter().enumerate().all(|(i, b)| if i == 4 || i == 7 { *b == b'-' } else { b.is_ascii_digit() });
    let at = if dated { parse_time(text) } else { duration_ms(text).and_then(|ms| now.checked_sub(ms)) };
    match at {
        Some(at) if at.abs() < 8_640_000_000_000_000 => Ok(iso(at)),
        _ => Err(format!("{name} takes an ISO time or a duration such as 1h, not \"{raw}\"")),
    }
}
