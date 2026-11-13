use crate::cpu::{self, CpuTimes, SAMPLES};
use crate::fail::{Fail, Result};
use crate::format::now_ms;
use crate::sys::{self, Stop};
use rusqlite::{Connection, OptionalExtension, params};
use std::path::{Path, PathBuf};
use std::time::Duration;

/// The least time between two heavy starts on the machine: long enough for the last one to show in the CPU usage.
const START_GAP_MS: i64 = 10_000;

/// The CPU usage, in %, a start waits to be under while others run.
pub const CPU_TARGET: u32 = 60;

/// How long a command waits to start before it gives up with 75.
pub const WAIT_SEC: u64 = 900;

/// Where heavy.db lives: `${XDG_STATE_HOME:-~/.local/state}/heavy-cmd`, `%LOCALAPPDATA%\heavy-cmd` on Windows.
pub fn state_dir() -> PathBuf {
    let (name, fallback) = if cfg!(windows) { ("LOCALAPPDATA", "AppData/Local") } else { ("XDG_STATE_HOME", ".local/state") };
    let base = std::env::var_os(name).map(PathBuf::from).filter(|dir| dir.is_absolute());
    base.unwrap_or_else(|| std::env::home_dir().unwrap_or_default().join(fallback)).join("heavy-cmd")
}

#[derive(Debug, Clone, PartialEq)]
pub struct HeavyStatus {
    /// The running commands' pids, oldest start first.
    pub running: Vec<u32>,
    /// The waiters' pids, in the order they start.
    pub waiting: Vec<u32>,
    /// The machine's CPU usage over the last 10 seconds, in %; None until it is measured.
    pub cpu: Option<f64>,
    /// When the last command started, in ms.
    pub last_start: Option<i64>,
}

/// Told once that a command waits, with the queue as it is.
pub type OnWait = Box<dyn FnMut(&HeavyStatus) + Send>;

/// How a command waits for its start.
pub struct ClaimOptions {
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
        ClaimOptions { wait_sec: WAIT_SEC, cpu: CPU_TARGET, poll_ms: 2_000, on_wait: None, stop: None }
    }
}

/// What the queue reads of the machine; a test's are fakes.
pub struct HeavyPorts {
    pub pid_alive: Box<dyn Fn(u32) -> bool + Send>,
    pub booted_at: Box<dyn Fn() -> i64 + Send>,
    pub now: Box<dyn Fn() -> i64 + Send>,
    pub sample: Box<dyn Fn() -> CpuTimes + Send>,
}

impl HeavyPorts {
    pub fn machine() -> HeavyPorts {
        HeavyPorts {
            pid_alive: Box::new(sys::pid_alive),
            booted_at: Box::new(sys::booted_at),
            now: Box::new(now_ms),
            sample: Box::new(sys::cpu_times),
        }
    }
}

const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS queue (id INTEGER PRIMARY KEY AUTOINCREMENT, pid INTEGER NOT NULL, since INTEGER NOT NULL, started_at INTEGER);
CREATE TABLE IF NOT EXISTS meta (key TEXT PRIMARY KEY, value INTEGER NOT NULL);
";

struct Row {
    id: i64,
    pid: u32,
    since: i64,
    started_at: Option<i64>,
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
        db.execute_batch(&format!("{SCHEMA}{SAMPLES};"))?;
        Ok(Heavy { db, ports })
    }

    pub fn now(&self) -> i64 {
        (self.ports.now)()
    }

    /// The rows whose process lives, oldest first; `prune` deletes the others. One from before the last
    /// boot names a pid the system may have reused.
    fn live(&self, prune: bool) -> Result<Vec<Row>> {
        let booted = (self.ports.booted_at)();
        let mut statement = self.db.prepare("SELECT id, pid, since, started_at FROM queue ORDER BY id")?;
        let rows =
            statement.query_map([], |row| Ok(Row { id: row.get(0)?, pid: row.get(1)?, since: row.get(2)?, started_at: row.get(3)? }))?;
        let mut live = Vec::new();
        for row in rows {
            let row = row?;
            if row.since >= booted && (self.ports.pid_alive)(row.pid) {
                live.push(row);
            } else if prune {
                self.db.execute("DELETE FROM queue WHERE id = ?1", [row.id])?;
            }
        }
        Ok(live)
    }

    fn last_start(&self) -> Result<Option<i64>> {
        Ok(self.db.query_row("SELECT value FROM meta WHERE key = 'last_start'", [], |row| row.get(0)).optional()?)
    }

    fn set_last_start(&self, at: i64) -> Result<()> {
        self.db.execute("INSERT OR REPLACE INTO meta (key, value) VALUES ('last_start', ?1)", [at])?;
        Ok(())
    }

    /// Starts waiter `id` when it is the oldest and both the gap and the CPU allow; whether it did.
    fn decide(&self, id: i64, target: u32, now: i64, times: CpuTimes) -> Result<bool> {
        let rows = self.live(true)?;
        let cpu = cpu::measure(&self.db, now, times)?;
        if rows.iter().find(|row| row.started_at.is_none()).is_none_or(|oldest| oldest.id != id) {
            return Ok(false);
        }
        if let Some(last) = self.last_start()? {
            if last > now {
                // A clock set back starts the gap again from now, rather than skipping it or waiting for the old time.
                self.set_last_start(now)?;
                return Ok(false);
            }
            if now - last < START_GAP_MS {
                return Ok(false);
            }
        }
        if rows.iter().any(|row| row.started_at.is_some()) && cpu.is_none_or(|cpu| cpu >= f64::from(target)) {
            return Ok(false);
        }
        self.db.execute("UPDATE queue SET started_at = ?1 WHERE id = ?2", params![now, id])?;
        self.set_last_start(now)?;
        Ok(true)
    }

    /// One write transaction per decision: two waiters never both start in the same gap.
    fn admit(&self, id: i64, target: u32) -> Result<bool> {
        let times = (self.ports.sample)();
        self.db.execute_batch("BEGIN IMMEDIATE")?;
        let decided = self.decide(id, target, self.now(), times);
        let ended = self.db.execute_batch(if decided.is_ok() { "COMMIT" } else { "ROLLBACK" });
        let started = decided?;
        ended?;
        Ok(started)
    }

    /// Queues a command as `pid`'s, and returns its id once it starts: a Busy failure when it could not
    /// within `wait_sec`, a Stopped one when its stop came first. Either way it no longer counts.
    pub fn claim(&self, options: &mut ClaimOptions, pid: u32) -> Result<i64> {
        let deadline = self.now() + options.wait_sec as i64 * 1000;
        self.db.execute("INSERT INTO queue (pid, since) VALUES (?1, ?2)", params![pid, self.now()])?;
        let id = self.db.last_insert_rowid();
        let claimed = self.wait_turn(id, options, deadline);
        if claimed.is_err() {
            let _ = self.release(id);
        }
        claimed.map(|()| id)
    }

    fn wait_turn(&self, id: i64, options: &mut ClaimOptions, deadline: i64) -> Result<()> {
        let mut told = false;
        loop {
            stopped(options.stop)?;
            if self.admit(id, options.cpu)? {
                return Ok(());
            }
            if self.now() + options.poll_ms as i64 > deadline {
                return Err(Fail::Busy(format!("no heavy command could start in {}s", options.wait_sec)));
            }
            if !told && let Some(on_wait) = options.on_wait.as_mut() {
                on_wait(&self.status()?);
            }
            told = true;
            pause(options.poll_ms, options.stop)?;
        }
    }

    /// Its command ended, or its wait did: it no longer counts.
    pub fn release(&self, id: i64) -> Result<()> {
        self.db.execute("DELETE FROM queue WHERE id = ?1", [id])?;
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
        // Ids are the order of arrival, which is the order of starts.
        let (running, waiting): (Vec<Row>, Vec<Row>) = self.live(false)?.into_iter().partition(|row| row.started_at.is_some());
        let pids = |rows: Vec<Row>| rows.into_iter().map(|row| row.pid).collect();
        Ok(HeavyStatus { running: pids(running), waiting: pids(waiting), cpu, last_start: self.last_start()? })
    }
}

/// A Stopped failure once `stop` is.
fn stopped(stop: Option<&Stop>) -> Result<()> {
    match stop.and_then(Stop::signalled) {
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
