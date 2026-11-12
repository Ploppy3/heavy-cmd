import { spawn, type ChildProcess } from "node:child_process";
import { constants } from "node:os";
import { SAMPLE_MS } from "./cpu.ts";
import { HeavyBusyError, type ClaimOptions, type Heavy } from "./queue.ts";

/** What heavy-cmd says on stderr while it waits to start: a stable text callers match. */
export const WAITING = "every heavy slot is busy";

/** Set for the command, to its id in the queue: a heavy command it starts runs within its admission instead of waiting on itself. */
export const INSIDE = "HEAVY_CMD_ID";

/** Who runs a command, when no `--agent` says it; handed on to the command, so one it starts is recorded as theirs too. */
export const AGENT = "HEAVY_CMD_AGENT";

/** The signals heavy-cmd acts on: while it waits they stop the wait, once its command runs they are the command's. */
const SIGNALS = ["SIGINT", "SIGTERM", "SIGHUP"] as const;

/** The exit of a process a signal ended, as a shell reports it. */
const signalExit = (signal: NodeJS.Signals) => 128 + constants.signals[signal];

/** A wait stopped by an abort whose reason is a signal's name exits as that signal ends a command; another abort, with none. */
const stoppedExit = (reason: unknown) => (typeof reason === "string" && Object.hasOwn(constants.signals, reason) ? signalExit(reason as NodeJS.Signals) : null);

/**
 * Runs `argv` once it may start and resolves to its exit code: one argument is
 * a shell line, several are the program and its arguments. Inside another
 * heavy command it runs straight away, since waiting on itself never ends. A
 * signal while it waits stops the wait, which is recorded, then ends this
 * process as it ended it before the record.
 */
export async function runHeavy(heavy: Heavy, argv: string[], options: ClaimOptions): Promise<number> {
  const stop = new AbortController();
  let child: ChildProcess | null = null;
  // The terminal's SIGINT reaches a running command through its process group; the others are passed on.
  const onSignal = (signal: NodeJS.Signals) => {
    if (!child) stop.abort(signal);
    else if (signal !== "SIGINT") child.kill(signal);
  };
  for (const signal of SIGNALS) process.on(signal, onSignal);
  const signal = options.signal ? AbortSignal.any([options.signal, stop.signal]) : stop.signal;
  try {
    return await heavily(heavy, { ...options, command: argv, signal }, (env) => exitOf(argv, env, (started) => (child = started)), (code) => code);
  } finally {
    for (const each of SIGNALS) process.off(each, onSignal);
    if (stop.signal.aborted) process.kill(process.pid, stop.signal.reason as NodeJS.Signals);
  }
}

/**
 * Runs `work` once a heavy command may start, holding its slot until `work`
 * settles; `env` is what a command it starts runs with. Inside another heavy
 * command it runs straight away. Once over it is recorded: run, with `exitOf`
 * its value as its exit or 1 when it throws; given up, with the code heavy-cmd
 * exits with; or stopped while it waited.
 */
export async function heavily<T>(
  heavy: Heavy,
  options: ClaimOptions,
  work: (env: NodeJS.ProcessEnv) => Promise<T>,
  exitOf: (value: T) => number = () => 0,
): Promise<T> {
  const queued_at = new Date(heavy.now()).toISOString();
  const inside = process.env[INSIDE];
  if (inside) {
    const id = /^\d+$/.test(inside) ? Number(inside) : null;
    return recorded(() => work(envOf(options)), exitOf, (exit) => heavy.record(options, { queued_at, started_at: queued_at, exit, outcome: "ran", inside: id }));
  }
  const holder = await heavy.claim(options).catch((error: unknown) => {
    const stopped = options.signal?.aborted ?? false;
    const exit = stopped ? stoppedExit(options.signal!.reason) : error instanceof HeavyBusyError ? error.code : 1;
    heavy.record(options, { queued_at, started_at: null, exit, outcome: stopped ? "stopped while waiting" : "gave up" });
    throw error;
  });
  const { started_at } = holder;
  // While it runs, its samples keep the machine's CPU reading fresh for the waiters.
  const sampling = setInterval(() => {
    try {
      heavy.sample();
    } catch {
      /* the next interval samples again */
    }
  }, SAMPLE_MS);
  try {
    return await recorded(() => work(envOf(options, holder.id)), exitOf, (exit) =>
      heavy.record(options, { queued_at, started_at, exit, outcome: "ran", heavy_id: holder.id }),
    );
  } finally {
    clearInterval(sampling);
    heavy.release(holder);
  }
}

/** This process's env for its command: its queue id, when it has one, and its agent. */
function envOf(options: ClaimOptions, id?: number): NodeJS.ProcessEnv {
  const env = { ...process.env };
  if (id !== undefined) env[INSIDE] = String(id);
  if (options.agent) env[AGENT] = options.agent;
  return env;
}

/** What `work` resolves with, recorded once it settles: `exitOf` its value, 1 when it throws. */
async function recorded<T>(work: () => Promise<T>, exitOf: (value: T) => number, record: (exit: number) => void): Promise<T> {
  let exit = 1;
  try {
    const value = await work();
    exit = exitOf(value);
    return value;
  } finally {
    record(exit);
  }
}

/** Runs the command, handing its process to `started`, and resolves to its exit code. */
function exitOf(argv: string[], env: NodeJS.ProcessEnv, started: (child: ChildProcess) => void): Promise<number> {
  const [file, ...args] = argv.length === 1 ? ["sh", "-c", argv[0]!] : argv;
  return new Promise((resolve) => {
    const child = spawn(file!, args, { stdio: "inherit", env });
    started(child);
    child.on("error", (error) => {
      process.stderr.write(`heavy-cmd: cannot run ${file}: ${error.message}\n`);
      resolve(127);
    });
    child.on("exit", (code, signal) => resolve(code ?? (signal ? signalExit(signal) : 128)));
  });
}
