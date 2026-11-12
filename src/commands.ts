import { integerFlag, json, one, UsageError, type Flags } from "./cli.ts";
import { loadConfig, stateDir } from "./config.ts";
import { usageLine } from "./cpu.ts";
import { clock, duration, pidAlive } from "./format.ts";
import { projectOf } from "./project.ts";
import { Heavy, readRuns, type HeavyStatus } from "./queue.ts";
import { AGENT, runHeavy, WAITING } from "./run.ts";
import { sinceOf, type HeavyRun } from "./runs.ts";

/** `use` on the machine's queue in `env`'s state folder, closed once the promise it returns settles. */
async function onQueue<T>(env: NodeJS.ProcessEnv, use: (heavy: Heavy) => Promise<T> | T): Promise<T> {
  const heavy = new Heavy(stateDir(env), { pidAlive });
  try {
    return await use(heavy);
  } finally {
    heavy.close();
  }
}

/** `2 running · cpu 63%/60%, last start 42s ago`. */
function summary(status: HeavyStatus, target: number, now: number): string {
  const last = status.last_start === null ? "no start yet" : `last start ${duration(now - Date.parse(status.last_start))} ago`;
  return `${usageLine(status.running.length, status.cpu, target)}, ${last}`;
}

/** The summary, then the running commands and the waiters, oldest first. */
export function statusLines(status: HeavyStatus, target: number, now: number): string[] {
  return [
    summary(status, target, now),
    ...status.running.map((h) => `running  ${h.project}: ${h.label} (pid ${h.pid}, since ${h.started_at})`),
    ...status.waiting.map((w, index) => `waiting  ${index + 1}. ${w.project}: ${w.label} (pid ${w.pid}, since ${w.since})`),
  ];
}

/** `14:02:11  heavy-cmd  npm run check  exit 0  waited 12s  ran 3m 4s  task-73`: when it ended, its date too on another day than `now`'s. */
export function runLine(run: HeavyRun, now = Date.now()): string {
  const at = clock(Date.parse(run.ended_at), now, true);
  const started = Date.parse(run.started_at ?? run.ended_at);
  const waited = duration(started - Date.parse(run.queued_at));
  const how =
    run.outcome === "ran"
      ? `exit ${run.exit ?? "?"}  waited ${waited}  ran ${duration(Date.parse(run.ended_at) - started)}`
      : `${run.outcome} after ${waited}${run.exit === null ? "" : ` (${run.exit})`}`;
  const inside = run.inside === null ? null : `inside heavy ${run.inside}`;
  return [at, run.project, run.label, how, run.agent, inside].filter((part) => part !== null).join("  ");
}

/**
 * `heavy-cmd -- <command>`: runs it once the machine has room, and resolves
 * to its exit. Its project is `--project`, else the cwd's (`projectOf`); its
 * agent `--agent`, else `HEAVY_CMD_AGENT`; its label `--label`, else the
 * command's first 80 characters.
 */
export async function cmdRun(command: string[], flags: Flags, env = process.env, cwd = process.cwd()): Promise<number> {
  const config = loadConfig(env, { wait: one(flags, "wait") });
  const project = one(flags, "project") || projectOf(cwd);
  return onQueue(env, (heavy) =>
    runHeavy(heavy, command, {
      project,
      label: one(flags, "label") || command.join(" ").slice(0, 80),
      agent: one(flags, "agent") || env[AGENT] || null,
      cwd,
      waitSec: config.wait,
      cpu: config.cpu,
      onWait: (status) => {
        const by = status.running.map((h) => `${h.project}: ${h.label}`).join(", ");
        const line = `${summary(status, config.cpu, Date.now())}${by ? ` (${by})` : ""}`;
        process.stderr.write(`heavy-cmd: ${WAITING}: ${line}; waiting up to ${config.wait}s\n`);
      },
    }),
  );
}

/** `heavy-cmd status [--json]`: the running commands, the waiters in order, the CPU against its target and the time since the last start. */
export async function cmdStatus(flags: Flags, env = process.env): Promise<void> {
  const config = loadConfig(env);
  const status = await onQueue(env, (heavy) => heavy.status());
  const now = Date.now();
  const since = status.last_start === null ? null : Math.round((now - Date.parse(status.last_start)) / 1000);
  json(flags, { ...status, target: config.cpu, since_last_start_sec: since }, () => statusLines(status, config.cpu, now).join("\n"));
}

/** `heavy-cmd logs`: the machine's record of heavy commands, or one project's, the last ended first, 50 unless `--limit`. */
export function cmdLogs(flags: Flags, env = process.env): void {
  const since = one(flags, "since");
  let from: string | undefined;
  try {
    from = since ? sinceOf(since, Date.now(), "--since") : undefined;
  } catch (error) {
    throw new UsageError((error as Error).message);
  }
  const filter = { project: one(flags, "project") || undefined, limit: integerFlag(flags, "limit", 50), since: from };
  const runs = readRuns(stateDir(env), filter);
  json(flags, runs, () => (runs.length ? runs.map((run) => runLine(run)).join("\n") : "no heavy command recorded"));
}
