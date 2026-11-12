import type { DatabaseSync } from "node:sqlite";

/** How a heavy command ended: it ran, or its wait ended before it could. */
export const OUTCOMES = ["ran", "gave up", "stopped while waiting"] as const;
export type Outcome = (typeof OUTCOMES)[number];

/** A heavy command once it is over. */
export type HeavyRun = {
  id: number;
  project: string;
  label: string;
  /** One argument is its `sh -c` line, several the program and its arguments; null for work that is not a command. */
  command: string[] | null;
  cwd: string;
  host: string;
  pid: number;
  /** The agent that ran it: `--agent`, else `HEAVY_CMD_AGENT`; null when neither is set. */
  agent: string | null;
  /** Its id in the queue, the `HEAVY_CMD_ID` its own commands see; null when it never started there. */
  heavy_id: number | null;
  /** The heavy command it ran inside, by that one's `heavy_id`: its `HEAVY_CMD_ID`. */
  inside: number | null;
  queued_at: string;
  /** Null when it never started. */
  started_at: string | null;
  ended_at: string;
  /** The command's exit code, `128 + signal` for a signal; null for a wait aborted otherwise than by a signal. */
  exit: number | null;
  outcome: Outcome;
};

export const RUNS = `
CREATE TABLE IF NOT EXISTS runs (
  id INTEGER PRIMARY KEY AUTOINCREMENT, project TEXT NOT NULL, label TEXT NOT NULL, command TEXT, cwd TEXT NOT NULL,
  host TEXT NOT NULL, pid INTEGER NOT NULL, agent TEXT, heavy_id INTEGER, inside INTEGER,
  queued_at TEXT NOT NULL, started_at TEXT, ended_at TEXT NOT NULL, exit INTEGER, outcome TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS runs_ended ON runs (ended_at);
`;

/** How long a run is kept once it is over. */
const KEEP_MS = 30 * 86_400_000;

/** Records `run`, and forgets the runs that ended over 30 days before `now`. */
export function recordRun(db: DatabaseSync, run: Omit<HeavyRun, "id">, now: number): void {
  db.prepare("DELETE FROM runs WHERE ended_at < ?").run(new Date(now - KEEP_MS).toISOString());
  db.prepare(
    `INSERT INTO runs (project, label, command, cwd, host, pid, agent, heavy_id, inside, queued_at, started_at, ended_at, exit, outcome)
     VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)`,
  ).run(
    run.project,
    run.label,
    run.command === null ? null : JSON.stringify(run.command),
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
    run.outcome,
  );
}

/** Which runs to read: one project's or the machine's, those that ended at `since` or later, the newest `limit`. */
export type RunFilter = { project?: string; limit: number; since?: string };

/** The runs `filter` keeps, the last ended first. */
export function runsOf(db: DatabaseSync, filter: RunFilter): HeavyRun[] {
  const where: string[] = [];
  const params: (string | number)[] = [];
  if (filter.project) {
    where.push("project = ?");
    params.push(filter.project);
  }
  if (filter.since) {
    where.push("ended_at >= ?");
    params.push(filter.since);
  }
  const sql = `SELECT * FROM runs ${where.length ? `WHERE ${where.join(" AND ")}` : ""} ORDER BY ended_at DESC, id DESC LIMIT ?`;
  const rows = db.prepare(sql).all(...params, filter.limit) as (Omit<HeavyRun, "command"> & { command: string | null })[];
  return rows.map((row) => ({ ...row, command: row.command === null ? null : (JSON.parse(row.command) as string[]) }));
}

const UNIT_MS: Record<string, number> = { s: 1000, m: 60_000, h: 3_600_000, d: 86_400_000 };

/** `since`: an ISO time, or a duration back from `now` such as `90s`, `15m`, `1h30m` or `2d`, as an ISO time; throws naming `name` on anything else. */
export function sinceOf(raw: string, now: number, name = "since"): string {
  const text = raw.trim();
  if (/^(?:\d+[smhd])+$/.test(text)) {
    const ms = [...text.matchAll(/(\d+)([smhd])/g)].reduce((sum, [, n, unit]) => sum + Number(n) * UNIT_MS[unit!]!, 0);
    return new Date(now - ms).toISOString();
  }
  const at = /^\d{4}-\d{2}-\d{2}/.test(text) ? Date.parse(text) : NaN;
  if (Number.isNaN(at)) throw new Error(`${name} takes an ISO time or a duration such as 1h, not "${raw}"`);
  return new Date(at).toISOString();
}
