use rusqlite::{Connection, OptionalExtension, params};

/// The CPU usage a start is judged by is the machine's over this window.
const CPU_WINDOW_MS: i64 = 10_000;

/// How often a sample is kept, whoever takes it: waiters on each poll, running commands, every status read.
pub const SAMPLE_MS: i64 = 2_000;

/// A window starting longer ago than this no longer measures the last 10 seconds.
const STALE_MS: i64 = 15_000;

/// Every core's time since boot, in ms: busy, and busy with idle.
#[derive(Debug, Default, Clone, Copy, PartialEq)]
pub struct CpuTimes {
    pub busy: f64,
    pub total: f64,
}

/// Busy over total between two readings, in %; None when the counters went back, as they do on a restart.
pub fn percent(then: CpuTimes, now: CpuTimes) -> Option<f64> {
    let total = now.total - then.total;
    let busy = now.busy - then.busy;
    if total <= 0.0 || busy < 0.0 {
        return None;
    }
    Some(((busy / total * 100.0).min(100.0) * 10.0).round() / 10.0)
}

pub const SAMPLES: &str = "CREATE TABLE IF NOT EXISTS samples (at INTEGER PRIMARY KEY, busy REAL NOT NULL, total REAL NOT NULL)";

/// The usage from the newest sample at least 10 seconds old to `times`: None while there is none, or it is stale. It writes nothing.
pub fn usage(db: &Connection, now: i64, times: CpuTimes) -> rusqlite::Result<Option<f64>> {
    let then = db
        .query_row(
            "SELECT busy, total FROM samples WHERE at <= ?1 AND at >= ?2 ORDER BY at DESC LIMIT 1",
            params![now - CPU_WINDOW_MS, now - STALE_MS],
            |row| Ok(CpuTimes { busy: row.get(0)?, total: row.get(1)? }),
        )
        .optional()?;
    Ok(then.and_then(|then| percent(then, times)))
}

/// Keeps `times` among the machine's samples, at most one every 2 seconds, and returns the usage. A
/// sample ahead of `now`, from a clock since set back, is dropped.
pub fn measure(db: &Connection, now: i64, times: CpuTimes) -> rusqlite::Result<Option<f64>> {
    db.execute("DELETE FROM samples WHERE at < ?1 OR at > ?2", params![now - 2 * STALE_MS, now])?;
    db.execute(
        "INSERT OR IGNORE INTO samples (at, busy, total) SELECT ?1, ?2, ?3 WHERE NOT EXISTS (SELECT 1 FROM samples WHERE at > ?4)",
        params![now, times.busy, times.total, now - SAMPLE_MS],
    )?;
    usage(db, now, times)
}

/// `2 running · cpu 63%/80%`, with `?` while the usage is not measured yet.
pub fn usage_line(running: usize, cpu: Option<f64>, target: u32) -> String {
    let cpu = cpu.map_or_else(|| "?".to_string(), |cpu| format!("{}%", cpu.round()));
    format!("{running} running · cpu {cpu}/{target}%")
}
