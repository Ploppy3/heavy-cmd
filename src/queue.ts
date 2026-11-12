import { existsSync, mkdirSync } from "node:fs";
import { hostname, uptime } from "node:os";
import path from "node:path";
import { setTimeout as wait } from "node:timers/promises";
import { DatabaseSync } from "node:sqlite";
import { cpuTimes, measure, SAMPLES, type CpuTimes } from "./cpu.ts";
import { recordRun, RUNS, runsOf, type HeavyRun, type RunFilter } from "./runs.ts";

/** The command could not start in time: sysexits' EX_TEMPFAIL, so it never reads as the command's own exit. */
export const NO_SLOT_EXIT = 75;

export class HeavyBusyError extends Error {
  readonly code: number;

  constructor(message: string, code = NO_SLOT_EXIT) {
    super(message);
    this.code = code;
  }
}

/** The least time between two heavy starts on the machine: long enough for the last one to show in the CPU usage. */
const START_GAP_MS = 10_000;

type Who = { id: number; pid: number; host: string; project: string; label: string };
type Holder = Who & { started_at: string };
type Row = Who & { since: string; started_at: string | null };

export type HeavyStatus = {
  /** Oldest start first. */
  running: Holder[];
  /** In the order they start: the oldest first. */
  waiting: (Who & { since: string })[];
  /** The machine's CPU usage over the last 10 seconds, in %; null until it is measured. */
  cpu: number | null;
  last_start: string | null;
};

export type ClaimOptions = {
  project: string;
  label: string;
  /** The command its record keeps whole: one argument a `sh -c` line, several its argv; none for work that is not a command. */
  command?: string[];
  /** Where it runs: this process's working directory unless set. */
  cwd?: string;
  /** Who ran it, as its record keeps it: `--agent`, else `HEAVY_CMD_AGENT`; none when neither is set. */
  agent?: string | null;
  waitSec: number;
  /** The CPU target: the usage, in %, a start waits to be under while others run. */
  cpu: number;
  /** Poll interval; 2000 in production, small in tests. */
  pollMs?: number;
  /** Called once, when the first pass cannot start. */
  onWait?: (status: HeavyStatus) => void;
  /** Ends the wait at once, throwing its reason. */
  signal?: AbortSignal;
};

export type HeavyPorts = {
  pidAlive: (pid: number) => boolean;
  bootedAt?: () => number;
  now?: () => number;
  sample?: () => CpuTimes;
};

const SCHEMA = `
CREATE TABLE IF NOT EXISTS commands (
  id INTEGER PRIMARY KEY AUTOINCREMENT, pid INTEGER NOT NULL, host TEXT NOT NULL,
  project TEXT NOT NULL, label TEXT NOT NULL, since TEXT NOT NULL, started_at TEXT
);
CREATE TABLE IF NOT EXISTS meta (key TEXT PRIMARY KEY, value TEXT NOT NULL);
${SAMPLES};
${RUNS}
`;

/** What a run's record takes from its command; the rest is how it ran. */
export type RunOf = Pick<ClaimOptions, "project" | "label" | "command" | "cwd" | "agent">;
type Ran = Pick<HeavyRun, "queued_at" | "started_at" | "exit" | "outcome"> & Partial<Pick<HeavyRun, "heavy_id" | "inside">>;

const sleep = (ms: number, signal?: AbortSignal) => wait(ms, undefined, { signal });
const who = ({ id, pid, host, project, label }: Row): Who => ({ id, pid, host, project, label });
const bootTime = () => Date.now() - uptime() * 1000;
const ROWS = "SELECT * FROM commands ORDER BY id";

/** Counted while its process lives. One from before the last boot names a pid the kernel may have reused. */
function alive(row: Row, booted: number, pidAlive: (pid: number) => boolean): boolean {
  if (row.host !== hostname()) return true; // another machine's process cannot be disproved
  if (Date.parse(row.since) < booted) return false;
  return pidAlive(row.pid);
}

function lastStartOf(db: DatabaseSync): number | null {
  const row = db.prepare("SELECT value FROM meta WHERE key = 'last_start'").get() as { value: string } | undefined;
  return row ? Number(row.value) : null;
}

function statusOf(rows: Row[], cpu: number | null, last: number | null): HeavyStatus {
  return {
    // Ids are the order of arrival, which is the order of starts.
    running: rows.filter((row) => row.started_at !== null).map((row) => ({ ...who(row), started_at: row.started_at! })),
    waiting: rows.filter((row) => row.started_at === null).map((row) => ({ ...who(row), since: row.since })),
    cpu,
    last_start: last === null ? null : new Date(last).toISOString(),
  };
}

/**
 * The machine's heavy commands, in one SQLite file, heavy.db: each joins
 * as a waiter and starts, oldest first, once the last start was 10 seconds ago
 * and, while others run, the CPU usage over the last 10 seconds is under its
 * target.
 */
export class Heavy {
  private db: DatabaseSync;
  private pidAlive: (pid: number) => boolean;
  private bootedAt: () => number;
  private times: () => CpuTimes;
  readonly now: () => number;

  /** A command belongs to the `heavy-cmd` process that runs it, so it stops counting when that process ends, however it ends. */
  constructor(dir: string, ports: HeavyPorts) {
    this.pidAlive = ports.pidAlive;
    this.bootedAt = ports.bootedAt ?? bootTime;
    this.now = ports.now ?? Date.now;
    this.times = ports.sample ?? cpuTimes;
    mkdirSync(dir, { recursive: true });
    this.db = new DatabaseSync(path.join(dir, "heavy.db"));
    // Pids and CPU samples need not outlive a power loss.
    this.db.exec("PRAGMA busy_timeout = 5000; PRAGMA journal_mode = WAL; PRAGMA synchronous = NORMAL;");
    this.db.exec(SCHEMA);
  }

  close(): void {
    this.db.close();
  }

  /** The rows whose process lives, oldest first; `prune` deletes the others. */
  private live(prune = false): Row[] {
    const booted = this.bootedAt();
    const remove = this.db.prepare("DELETE FROM commands WHERE id = ?");
    return (this.db.prepare(ROWS).all() as Row[]).filter((row) => {
      if (alive(row, booted, this.pidAlive)) return true;
      if (prune) remove.run(row.id);
      return false;
    });
  }

  private setLastStart(at: number): void {
    this.db.prepare("INSERT OR REPLACE INTO meta (key, value) VALUES ('last_start', ?)").run(String(at));
  }

  /** Starts waiter `id` when it is the oldest and both the gap and the CPU allow; null otherwise. */
  private decide(id: number, target: number, now: number, times: CpuTimes): Holder | null {
    const rows = this.live(true);
    const cpu = measure(this.db, now, times);
    const oldest = rows.find((row) => row.started_at === null);
    if (oldest?.id !== id) return null;
    const last = lastStartOf(this.db);
    if (last !== null && last > now) {
      // A clock set back starts the gap again from now, rather than skipping it or waiting for the old time.
      this.setLastStart(now);
      return null;
    }
    if (last !== null && now - last < START_GAP_MS) return null;
    if (rows.some((row) => row.started_at !== null) && (cpu === null || cpu >= target)) return null;
    const started_at = new Date(now).toISOString();
    this.db.prepare("UPDATE commands SET started_at = ? WHERE id = ?").run(started_at, id);
    this.setLastStart(now);
    return { ...who(oldest), started_at };
  }

  /** One write transaction per decision: two waiters never both start in the same gap. */
  private admit(id: number, target: number): Holder | null {
    const times = this.times();
    this.db.exec("BEGIN IMMEDIATE");
    try {
      const holder = this.decide(id, target, this.now(), times);
      this.db.exec("COMMIT");
      return holder;
    } catch (error) {
      this.db.exec("ROLLBACK");
      throw error;
    }
  }

  /** Joins the queue as its youngest waiter, and returns its id. */
  join(command: { project: string; label: string }, pid = process.pid): number {
    const { lastInsertRowid } = this.db
      .prepare("INSERT INTO commands (pid, host, project, label, since) VALUES (?, ?, ?, ?, ?)")
      .run(pid, hostname(), command.project, command.label, new Date(this.now()).toISOString());
    return Number(lastInsertRowid);
  }

  async claim(options: ClaimOptions, pid = process.pid): Promise<Holder> {
    const pollMs = options.pollMs ?? 2000;
    const deadline = this.now() + options.waitSec * 1000;
    const id = this.join(options, pid);
    let told = false;
    try {
      for (;;) {
        options.signal?.throwIfAborted();
        const holder = this.admit(id, options.cpu);
        if (holder) return holder;
        if (this.now() + pollMs > deadline) {
          throw new HeavyBusyError(options.waitSec === 0 ? "no heavy command may start now" : `no heavy command could start in ${options.waitSec}s`);
        }
        if (!told) options.onWait?.(this.status());
        told = true;
        await sleep(pollMs, options.signal);
      }
    } catch (error) {
      this.release({ id });
      throw error;
    }
  }

  /** This process's run of `command`, over now. A write that fails says one line on stderr, and nothing else. */
  record(command: RunOf, ran: Ran): void {
    try {
      const now = this.now();
      recordRun(
        this.db,
        {
          project: command.project,
          label: command.label,
          command: command.command ?? null,
          cwd: command.cwd ?? process.cwd(),
          host: hostname(),
          pid: process.pid,
          agent: command.agent || null,
          ...ran,
          heavy_id: ran.heavy_id ?? null,
          inside: ran.inside ?? null,
          ended_at: new Date(now).toISOString(),
        },
        now,
      );
    } catch (error) {
      process.stderr.write(`heavy-cmd: the heavy command's run was not recorded: ${error instanceof Error ? error.message : String(error)}\n`);
    }
  }

  /** Its command ended, or its wait did: it no longer counts. */
  release(holder: { id: number }): void {
    this.db.prepare("DELETE FROM commands WHERE id = ?").run(holder.id);
  }

  /** Keeps a CPU sample: a running command's keep the reading fresh for those waiting. */
  sample(): void {
    measure(this.db, this.now(), this.times());
  }

  /** The running commands, the waiters and the CPU usage, keeping a sample as it reads. */
  status(): HeavyStatus {
    const cpu = measure(this.db, this.now(), this.times());
    return statusOf(this.live(), cpu, lastStartOf(this.db));
  }
}

/**
 * The runs `filter` keeps of `dir`'s record, read from heavy.db opened
 * read-only: nothing written, no CPU sample kept, no row pruned. None before
 * the record.
 */
export function readRuns(dir: string, filter: RunFilter): HeavyRun[] {
  const file = path.join(dir, "heavy.db");
  if (!existsSync(file)) return [];
  const db = new DatabaseSync(file, { readOnly: true });
  try {
    const tables = new Set(db.prepare("SELECT name FROM sqlite_master WHERE type = 'table'").all().map((row) => row.name));
    return tables.has("runs") ? runsOf(db, filter) : [];
  } finally {
    db.close();
  }
}
