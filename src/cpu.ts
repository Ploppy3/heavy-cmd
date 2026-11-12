import { cpus } from "node:os";
import type { DatabaseSync } from "node:sqlite";

/** The CPU usage a start is judged by is the machine's over this window. */
const CPU_WINDOW_MS = 10_000;

/** How often a sample is kept, whoever takes it: waiters on each poll, running commands, every status read. */
export const SAMPLE_MS = 2_000;

/** A window starting longer ago than this no longer measures the last 10 seconds. */
const STALE_MS = 15_000;

/** Every core's time since boot, in ms: busy, and busy with idle. */
export type CpuTimes = { busy: number; total: number };

export function cpuTimes(): CpuTimes {
  let busy = 0;
  let total = 0;
  for (const { times } of cpus()) {
    const used = times.user + times.nice + times.sys + times.irq;
    busy += used;
    total += used + times.idle;
  }
  return { busy, total };
}

/** Busy over total between two readings, in %; null when the counters went back, as they do on a restart. */
export function percent(then: CpuTimes, now: CpuTimes): number | null {
  const total = now.total - then.total;
  const busy = now.busy - then.busy;
  if (total <= 0 || busy < 0) return null;
  return Math.round(Math.min(100, (busy / total) * 100) * 10) / 10;
}

export const SAMPLES = "CREATE TABLE IF NOT EXISTS samples (at INTEGER PRIMARY KEY, busy REAL NOT NULL, total REAL NOT NULL)";

/**
 * The usage from the newest sample at least CPU_WINDOW_MS old to `times`:
 * null while there is none, or it is stale. It writes nothing.
 */
export function usage(db: DatabaseSync, now: number, times: CpuTimes): number | null {
  const then = db
    .prepare("SELECT busy, total FROM samples WHERE at <= ? AND at >= ? ORDER BY at DESC LIMIT 1")
    .get(now - CPU_WINDOW_MS, now - STALE_MS) as CpuTimes | undefined;
  return then ? percent(then, times) : null;
}

/**
 * Keeps `times` among the machine's samples, at most one every SAMPLE_MS, and
 * returns the usage. A sample ahead of `now`, from a clock since set back, is
 * dropped.
 */
export function measure(db: DatabaseSync, now: number, times: CpuTimes): number | null {
  db.prepare("DELETE FROM samples WHERE at < ? OR at > ?").run(now - 2 * STALE_MS, now);
  db.prepare("INSERT OR IGNORE INTO samples (at, busy, total) SELECT ?, ?, ? WHERE NOT EXISTS (SELECT 1 FROM samples WHERE at > ?)").run(
    now,
    times.busy,
    times.total,
    now - SAMPLE_MS,
  );
  return usage(db, now, times);
}

/** `2 running · cpu 63%/80%`, with `?` while the usage is not measured yet. */
export function usageLine(running: number, cpu: number | null, target: number): string {
  return `${running} running · cpu ${cpu === null ? "?" : `${Math.round(cpu)}%`}/${target}%`;
}
