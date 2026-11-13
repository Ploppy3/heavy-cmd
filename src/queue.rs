use crate::cpu::{self, CpuTimes, SAMPLES};
use crate::fail::{Fail, Result};
use crate::format::{iso, now_ms, parse_iso, warn};
use crate::runs::{HeavyRun, RUNS, RunFilter, record_run, runs_of};
use crate::sys::{self, Stop};
use rusqlite::{Connection, OpenFlags, OptionalExtension, params};
use serde_json::{Map, Value};
use std::path::Path;
use std::sync::OnceLock;
use std::time::Duration;

/// The least time between two heavy starts on the machine: long enough for the last one to show in the CPU usage.
const START_GAP_MS: i64 = 10_000;

/// A command in the queue: who runs it, and what.
#[derive(Debug, Clone, PartialEq)]
pub struct Who {
    pub id: i64,
    pub pid: u32,
    pub host: String,
    pub project: String,
    pub label: String,
}

impl Who {
    fn to_json(&self, at_key: &str, at: &str) -> Value {
        let mut who = Map::new();
        who.insert("id".into(), self.id.into());
        who.insert("pid".into(), self.pid.into());
        who.insert("host".into(), self.host.clone().into());
        who.insert("project".into(), self.project.clone().into());
        who.insert("label".into(), self.label.clone().into());
        who.insert(at_key.into(), at.into());
        Value::Object(who)
    }
}

/// A command that started, and when.
#[derive(Debug, Clone, PartialEq)]
pub struct Holder {
    pub who: Who,
    pub started_at: String,
}

/// A command waiting to start, since when.
#[derive(Debug, Clone, PartialEq)]
pub struct Waiter {
    pub who: Who,
    pub since: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct HeavyStatus {
    /// Oldest start first.
    pub running: Vec<Holder>,
    /// In the order they start: the oldest first.
    pub waiting: Vec<Waiter>,
    /// The machine's CPU usage over the last 10 seconds, in %; None until it is measured.
    pub cpu: Option<f64>,
    pub last_start: Option<String>,
}

impl HeavyStatus {
    pub fn to_json(&self) -> Map<String, Value> {
        let mut status = Map::new();
        status.insert("running".into(), self.running.iter().map(|h| h.who.to_json("started_at", &h.started_at)).collect());
        status.insert("waiting".into(), self.waiting.iter().map(|w| w.who.to_json("since", &w.since)).collect());
        status.insert("cpu".into(), self.cpu.map_or(Value::Null, crate::cli::number));
        status.insert("last_start".into(), self.last_start.clone().into());
        status
    }
}

/// Told once that a command waits, with the queue as it is.
pub type OnWait = Box<dyn FnMut(&HeavyStatus) + Send>;

/// A command to queue, and how it waits.
pub struct ClaimOptions {
    pub project: String,
    pub label: String,
    /// The command its record keeps whole: one argument a shell line, several its argv; none for work that is not a command.
    pub command: Option<Vec<String>>,
    /// Where it runs, as its record keeps it: this process's working directory unless set.
    pub cwd: Option<String>,
    /// Who ran it, as its record keeps it: `--agent`, else `HEAVY_CMD_AGENT`; none when neither is set.
    pub agent: Option<String>,
    /// The `HEAVY_CMD_ID` it was started with: inside that heavy command, it runs at once.
    pub inside: Option<String>,
    pub wait_sec: u64,
    /// The CPU target: the usage, in %, a start waits to be under while others run.
    pub cpu: u32,
    /// Poll interval; 2000 in production, small in tests.
    pub poll_ms: u64,
    /// Called once, when the first pass cannot start.
    pub on_wait: Option<OnWait>,
    /// Ends the wait once stopped.
    pub stop: Option<&'static Stop>,
}

impl Default for ClaimOptions {
    fn default() -> Self {
        ClaimOptions {
            project: String::new(),
            label: String::new(),
            command: None,
            cwd: None,
            agent: None,
            inside: None,
            wait_sec: 0,
            cpu: 60,
            poll_ms: 2_000,
            on_wait: None,
            stop: None,
        }
    }
}

/// How a run went, for its record.
#[derive(Debug, Clone, Default)]
pub struct Ran {
    pub queued_at: String,
    pub started_at: Option<String>,
    pub exit: Option<i64>,
    pub outcome: &'static str,
    pub heavy_id: Option<i64>,
    pub inside: Option<i64>,
}

/// What the queue reads of the machine; a test's are fakes.
pub struct HeavyPorts {
    pub pid_alive: Box<dyn Fn(u32) -> bool + Send>,
    pub booted_at: Box<dyn Fn() -> i64 + Send>,
    pub now: Box<dyn Fn() -> i64 + Send>,
    pub sample: Box<dyn Fn() -> CpuTimes + Send>,
    /// A line on stderr.
    pub warn: Box<dyn Fn(&str) + Send>,
    /// This process's pid, as its commands are queued and recorded.
    pub pid: u32,
}

impl HeavyPorts {
    pub fn machine() -> HeavyPorts {
        HeavyPorts {
            pid_alive: Box::new(sys::pid_alive),
            booted_at: Box::new(sys::booted_at),
            now: Box::new(now_ms),
            sample: Box::new(sys::cpu_times),
            warn: Box::new(warn),
            pid: std::process::id(),
        }
    }
}

/// This machine's name, as its rows carry it.
pub fn hostname() -> &'static str {
    static NAME: OnceLock<String> = OnceLock::new();
    NAME.get_or_init(sys::hostname)
}

const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS commands (
  id INTEGER PRIMARY KEY AUTOINCREMENT, pid INTEGER NOT NULL, host TEXT NOT NULL,
  project TEXT NOT NULL, label TEXT NOT NULL, since TEXT NOT NULL, started_at TEXT
);
CREATE TABLE IF NOT EXISTS meta (key TEXT PRIMARY KEY, value TEXT NOT NULL);
";

struct Row {
    who: Who,
    since: String,
    started_at: Option<String>,
}

/// Counted while its process lives. One from before the last boot names a pid the system may have reused.
fn alive(row: &Row, booted: i64, pid_alive: &dyn Fn(u32) -> bool) -> bool {
    if row.who.host != hostname() {
        return true; // another machine's process cannot be disproved
    }
    if parse_iso(&row.since).is_some_and(|since| since < booted) {
        return false;
    }
    pid_alive(row.who.pid)
}

/// The machine's heavy commands, in one SQLite file, heavy.db: each joins as a waiter and starts, oldest
/// first, once the last start was 10 seconds ago and, while others run, the CPU usage over the last 10
/// seconds is under its target. A command belongs to the heavy-cmd process that runs it, so it stops
/// counting when that process ends, however it ends.
pub struct Heavy {
    db: Connection,
    ports: HeavyPorts,
}

impl Heavy {
    pub fn open(dir: &Path, ports: HeavyPorts) -> Result<Heavy> {
        std::fs::create_dir_all(dir).map_err(|error| Fail::Other(format!("{}: {error}", dir.display())))?;
        let db = Connection::open(dir.join("heavy.db"))?;
        db.busy_timeout(Duration::from_secs(5))?;
        // Pids and CPU samples need not outlive a power loss.
        db.execute_batch("PRAGMA journal_mode = WAL; PRAGMA synchronous = NORMAL;")?;
        db.execute_batch(&format!("{SCHEMA}{SAMPLES};{RUNS}"))?;
        Ok(Heavy { db, ports })
    }

    pub fn now(&self) -> i64 {
        (self.ports.now)()
    }

    pub fn pid(&self) -> u32 {
        self.ports.pid
    }

    pub fn warn(&self, line: &str) {
        (self.ports.warn)(line)
    }

    /// The rows whose process lives, oldest first; `prune` deletes the others.
    fn live(&self, prune: bool) -> Result<Vec<Row>> {
        let booted = (self.ports.booted_at)();
        let mut statement = self.db.prepare("SELECT id, pid, host, project, label, since, started_at FROM commands ORDER BY id")?;
        let rows = statement.query_map([], |row| {
            Ok(Row {
                who: Who { id: row.get(0)?, pid: row.get(1)?, host: row.get(2)?, project: row.get(3)?, label: row.get(4)? },
                since: row.get(5)?,
                started_at: row.get(6)?,
            })
        })?;
        let mut live = Vec::new();
        for row in rows {
            let row = row?;
            if alive(&row, booted, &*self.ports.pid_alive) {
                live.push(row);
            } else if prune {
                self.db.execute("DELETE FROM commands WHERE id = ?1", [row.who.id])?;
            }
        }
        Ok(live)
    }

    fn last_start(&self) -> Result<Option<i64>> {
        let value: Option<String> =
            self.db.query_row("SELECT value FROM meta WHERE key = 'last_start'", [], |row| row.get(0)).optional()?;
        Ok(value.and_then(|value| value.parse().ok()))
    }

    fn set_last_start(&self, at: i64) -> Result<()> {
        self.db.execute("INSERT OR REPLACE INTO meta (key, value) VALUES ('last_start', ?1)", [at.to_string()])?;
        Ok(())
    }

    /// Starts waiter `id` when it is the oldest and both the gap and the CPU allow; None otherwise.
    fn decide(&self, id: i64, target: u32, now: i64, times: CpuTimes) -> Result<Option<Holder>> {
        let rows = self.live(true)?;
        let cpu = cpu::measure(&self.db, now, times)?;
        let Some(oldest) = rows.iter().find(|row| row.started_at.is_none()).filter(|row| row.who.id == id) else { return Ok(None) };
        if let Some(last) = self.last_start()? {
            if last > now {
                // A clock set back starts the gap again from now, rather than skipping it or waiting for the old time.
                self.set_last_start(now)?;
                return Ok(None);
            }
            if now - last < START_GAP_MS {
                return Ok(None);
            }
        }
        if rows.iter().any(|row| row.started_at.is_some()) && cpu.is_none_or(|cpu| cpu >= f64::from(target)) {
            return Ok(None);
        }
        let started_at = iso(now);
        self.db.execute("UPDATE commands SET started_at = ?1 WHERE id = ?2", params![started_at, id])?;
        self.set_last_start(now)?;
        Ok(Some(Holder { who: oldest.who.clone(), started_at }))
    }

    /// One write transaction per decision: two waiters never both start in the same gap.
    fn admit(&self, id: i64, target: u32) -> Result<Option<Holder>> {
        let times = (self.ports.sample)();
        self.db.execute_batch("BEGIN IMMEDIATE")?;
        let decided = self.decide(id, target, self.now(), times);
        let ended = self.db.execute_batch(if decided.is_ok() { "COMMIT" } else { "ROLLBACK" });
        let holder = decided?;
        ended?;
        Ok(holder)
    }

    /// Joins the queue as its youngest waiter, and returns its id.
    pub fn join(&self, project: &str, label: &str, pid: u32) -> Result<i64> {
        self.db.execute(
            "INSERT INTO commands (pid, host, project, label, since) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![pid, hostname(), project, label, iso(self.now())],
        )?;
        Ok(self.db.last_insert_rowid())
    }

    /// Queues `options`' command as `pid`'s, and returns once it starts: a Busy failure when it could not
    /// within `wait_sec`, a Stopped one when its stop came first. Either way it no longer counts.
    pub fn claim(&self, options: &mut ClaimOptions, pid: u32) -> Result<Holder> {
        let deadline = self.now() + options.wait_sec as i64 * 1000;
        let id = self.join(&options.project, &options.label, pid)?;
        let claimed = self.wait_turn(id, options, deadline);
        if claimed.is_err() {
            let _ = self.release(id);
        }
        claimed
    }

    fn wait_turn(&self, id: i64, options: &mut ClaimOptions, deadline: i64) -> Result<Holder> {
        let mut told = false;
        loop {
            stopped(options.stop)?;
            if let Some(holder) = self.admit(id, options.cpu)? {
                return Ok(holder);
            }
            if self.now() + options.poll_ms as i64 > deadline {
                return Err(Fail::Busy(if options.wait_sec == 0 {
                    "no heavy command may start now".into()
                } else {
                    format!("no heavy command could start in {}s", options.wait_sec)
                }));
            }
            if !told && let Some(on_wait) = options.on_wait.as_mut() {
                on_wait(&self.status()?);
            }
            told = true;
            pause(options.poll_ms, options.stop)?;
        }
    }

    /// This process's run of `options`' command, over now. A write that fails says one line on stderr, and nothing else.
    pub fn record(&self, options: &ClaimOptions, ran: Ran) {
        let now = self.now();
        let cwd = options.cwd.clone().unwrap_or_else(|| std::env::current_dir().map(|dir| dir.display().to_string()).unwrap_or_default());
        let run = HeavyRun {
            id: 0,
            project: options.project.clone(),
            label: options.label.clone(),
            command: options.command.clone(),
            cwd,
            host: hostname().to_string(),
            pid: i64::from(self.ports.pid),
            agent: options.agent.clone().filter(|agent| !agent.is_empty()),
            heavy_id: ran.heavy_id,
            inside: ran.inside,
            queued_at: ran.queued_at,
            started_at: ran.started_at,
            ended_at: iso(now),
            exit: ran.exit,
            outcome: ran.outcome.to_string(),
        };
        if let Err(error) = record_run(&self.db, &run, now) {
            self.warn(&format!("heavy-cmd: the heavy command's run was not recorded: {error}"));
        }
    }

    /// Its command ended, or its wait did: it no longer counts.
    pub fn release(&self, id: i64) -> Result<()> {
        self.db.execute("DELETE FROM commands WHERE id = ?1", [id])?;
        Ok(())
    }

    /// Keeps a CPU sample: a running command's keep the reading fresh for those waiting.
    pub fn sample(&self) -> Result<()> {
        cpu::measure(&self.db, self.now(), (self.ports.sample)())?;
        Ok(())
    }

    /// The running commands, the waiters and the CPU usage, keeping a sample as it reads.
    pub fn status(&self) -> Result<HeavyStatus> {
        let cpu = cpu::measure(&self.db, self.now(), (self.ports.sample)())?;
        let rows = self.live(false)?;
        let last_start = self.last_start()?.map(iso);
        // Ids are the order of arrival, which is the order of starts.
        let (running, waiting): (Vec<Row>, Vec<Row>) = rows.into_iter().partition(|row| row.started_at.is_some());
        Ok(HeavyStatus {
            running: running.into_iter().map(|row| Holder { who: row.who, started_at: row.started_at.unwrap_or_default() }).collect(),
            waiting: waiting.into_iter().map(|row| Waiter { who: row.who, since: row.since }).collect(),
            cpu,
            last_start,
        })
    }
}

/// A Stopped failure once `stop` is.
fn stopped(stop: Option<&Stop>) -> Result<()> {
    match stop.and_then(Stop::reason) {
        Some(signal) => Err(Fail::Stopped(signal)),
        None => Ok(()),
    }
}

/// Sleeps `ms`, waking at once when `stop` is.
fn pause(ms: u64, stop: Option<&Stop>) -> Result<()> {
    let mut left = ms;
    while left > 0 {
        let step = left.min(50);
        std::thread::sleep(Duration::from_millis(step));
        left -= step;
        stopped(stop)?;
    }
    Ok(())
}

/// The runs `filter` keeps of `dir`'s record, read from heavy.db opened read-only: nothing written, no
/// CPU sample kept, no row pruned. None before the record.
pub fn read_runs(dir: &Path, filter: &RunFilter) -> Result<Vec<HeavyRun>> {
    let file = dir.join("heavy.db");
    if !file.exists() {
        return Ok(Vec::new());
    }
    let db = Connection::open_with_flags(file, OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX)?;
    db.busy_timeout(Duration::from_secs(5))?;
    let has_runs: bool =
        db.query_row("SELECT EXISTS (SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = 'runs')", [], |row| row.get(0))?;
    if !has_runs {
        return Ok(Vec::new());
    }
    Ok(runs_of(&db, filter)?)
}
