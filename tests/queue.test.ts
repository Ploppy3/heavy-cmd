import { afterEach, beforeEach, describe, expect, test, vi } from "vitest";
import { existsSync, mkdtempSync, readFileSync, rmSync } from "node:fs";
import { hostname, tmpdir } from "node:os";
import path from "node:path";
import { DatabaseSync } from "node:sqlite";
import { cmdLogs, runLine, statusLines } from "../src/commands.ts";
import { Heavy, HeavyBusyError, readRuns, type ClaimOptions, type HeavyStatus } from "../src/queue.ts";
import { heavily, INSIDE, runHeavy } from "../src/run.ts";
import type { HeavyRun } from "../src/runs.ts";

let dir: string;
let alive: Set<number>;
let bootedAt: number;
/** The queue's clock, which only the test moves. */
let clock: number;
/** The machine's CPU usage, in %, from the last sample on. */
let cpu: number;
let times: { at: number; busy: number; total: number };
const pools: Heavy[] = [];

function sample() {
  const elapsed = clock - times.at;
  times = { at: clock, busy: times.busy + (elapsed * cpu) / 100, total: times.total + elapsed };
  return { busy: times.busy, total: times.total };
}

/** One process's view of the queue: each its own connection to the same file. */
function heavy(): Heavy {
  const pool = new Heavy(dir, { pidAlive: (pid) => alive.has(pid), bootedAt: () => bootedAt, now: () => clock, sample });
  pools.push(pool);
  return pool;
}

const options = (overrides: Partial<ClaimOptions> = {}): ClaimOptions => ({
  project: "api",
  label: "npm test",
  agent: "task-73",
  waitSec: 0,
  cpu: 80,
  pollMs: 1,
  ...overrides,
});

const claim = (h: Heavy, pid: number, overrides: Partial<ClaimOptions> = {}) => {
  alive.add(pid);
  return h.claim(options({ label: `cmd ${pid}`, ...overrides }), pid);
};

const advance = (ms: number) => void (clock += ms);
const tick = () => new Promise((resolve) => setTimeout(resolve, 20));
const pids = (status: HeavyStatus) => ({ running: status.running.map((h) => h.pid), waiting: status.waiting.map((w) => w.pid) });

beforeEach(() => {
  dir = mkdtempSync(path.join(tmpdir(), "heavy-cmd-queue-"));
  alive = new Set([process.pid]);
  bootedAt = 0;
  clock = 1_000_000;
  cpu = 0;
  times = { at: clock, busy: 0, total: 0 };
  delete process.env[INSIDE];
});

afterEach(() => {
  for (const pool of pools.splice(0)) pool.close();
  rmSync(dir, { recursive: true, force: true });
  delete process.env[INSIDE];
  vi.restoreAllMocks();
});

describe("admission", () => {
  test("with none running and no start yet, a command starts at once", async () => {
    const h = heavy();
    expect((await claim(h, 101)).pid).toBe(101);
    expect(pids(h.status())).toEqual({ running: [101], waiting: [] });
  });

  test("under the CPU target, a second command starts 10 seconds after the first, while the first runs", async () => {
    const h = heavy();
    cpu = 30;
    await claim(h, 101);
    advance(10_000);
    expect((await claim(h, 102)).pid).toBe(102);
    expect(pids(h.status()).running).toEqual([101, 102]);
  });

  test("over the CPU target with one running, a command waits, then gives up with nothing left behind", async () => {
    const h = heavy();
    cpu = 95;
    await claim(h, 101);
    advance(10_000);
    const busy = claim(h, 102);
    await expect(busy).rejects.toBeInstanceOf(HeavyBusyError);
    // heavy-cmd exits with it: sysexits' EX_TEMPFAIL, never the command's own code.
    await expect(busy).rejects.toHaveProperty("code", 75);
    expect(pids(h.status())).toEqual({ running: [101], waiting: [] });
    cpu = 50;
    advance(10_000);
    expect((await claim(h, 103)).pid).toBe(103);
  });

  test("a CPU usage not measured yet counts as over the CPU target while one runs", async () => {
    const h = heavy();
    await claim(h, 101);
    advance(16_000); // the only sample is stale: no reading over the last 10 seconds
    await expect(claim(h, 102)).rejects.toBeInstanceOf(HeavyBusyError);
  });

  test("10 seconds pass between two starts, whatever the CPU and whether the first still runs", async () => {
    const h = heavy();
    const first = await claim(h, 101);
    advance(9_999);
    await expect(claim(h, 102)).rejects.toBeInstanceOf(HeavyBusyError);
    h.release(first);
    await expect(claim(h, 102)).rejects.toBeInstanceOf(HeavyBusyError);
    advance(1);
    expect((await claim(h, 102)).pid).toBe(102);
  });

  test("a clock set back starts the gap again from then, without skipping it", async () => {
    const h = heavy();
    h.release(await claim(h, 101));
    advance(-60_000);
    await expect(claim(h, 102)).rejects.toBeInstanceOf(HeavyBusyError);
    advance(9_999);
    await expect(claim(h, 102)).rejects.toBeInstanceOf(HeavyBusyError);
    advance(1);
    expect((await claim(h, 102)).pid).toBe(102);
  });

  test("with none running, the oldest waiter starts once the gap has passed, even at 100% CPU", async () => {
    const h = heavy();
    cpu = 100;
    h.release(await claim(h, 101));
    const waiting = claim(h, 102, { waitSec: 60 });
    await tick();
    expect(pids(h.status()).waiting).toEqual([102]);
    advance(10_000);
    expect((await waiting).pid).toBe(102);
  });

  test("two waiters in two processes never start within the same gap, and the oldest starts first", async () => {
    const [a, b, c] = [heavy(), heavy(), heavy()];
    await claim(a, 101);
    const older = claim(b, 102, { waitSec: 60 });
    await tick();
    const younger = claim(c, 103, { waitSec: 60 });
    await tick();
    expect(pids(a.status()).waiting).toEqual([102, 103]);
    advance(10_000);
    expect((await older).pid).toBe(102);
    await tick();
    expect(pids(a.status())).toEqual({ running: [101, 102], waiting: [103] });
    advance(10_000);
    expect((await younger).pid).toBe(103);
  });

  test("a waiter whose process died no longer holds the queue", async () => {
    const h = heavy();
    h.release(await claim(h, 101));
    const dead = claim(h, 102, { waitSec: 15 });
    await tick();
    const next = claim(h, 103, { waitSec: 60 });
    alive.delete(102);
    advance(10_000);
    expect((await next).pid).toBe(103);
    advance(10_000);
    await expect(dead).rejects.toBeInstanceOf(HeavyBusyError);
  });

  test("a holder whose process is gone, or which predates the last boot, no longer counts as running", async () => {
    const h = heavy();
    cpu = 100;
    await claim(h, 101);
    alive.delete(101);
    advance(10_000);
    expect((await claim(h, 102)).pid).toBe(102);
    expect(pids(h.status()).running).toEqual([102]);
    bootedAt = clock + 1;
    advance(10_000);
    expect((await claim(h, 103)).pid).toBe(103);
    expect(pids(h.status()).running).toEqual([103]);
  });

  test("a wait is said once, with the running commands and the CPU usage", async () => {
    const h = heavy();
    cpu = 90;
    await claim(h, 101);
    const told: HeavyStatus[] = [];
    const waiting = claim(h, 102, { waitSec: 60, onWait: (status) => told.push(status) });
    await tick();
    advance(10_000);
    await tick();
    cpu = 10;
    advance(10_000);
    expect((await waiting).pid).toBe(102);
    expect(told).toHaveLength(1);
    expect(pids(told[0]!)).toEqual({ running: [101], waiting: [102] });
    expect(told[0]!.last_start).toBe(new Date(1_000_000).toISOString());
  });

  test("status shows the summary, the running commands and the waiters in order", async () => {
    const h = heavy();
    cpu = 85;
    await claim(h, 101);
    const waiting = claim(h, 102, { waitSec: 60 });
    await tick();
    advance(12_000);
    const status = h.status();
    expect(status.cpu).toBe(85);
    expect(statusLines(status, 80, clock)).toEqual([
      "1 running · cpu 85%/80%, last start 12s ago",
      `running  api: cmd 101 (pid 101, since ${new Date(1_000_000).toISOString()})`,
      `waiting  1. api: cmd 102 (pid 102, since ${new Date(1_000_000).toISOString()})`,
    ]);
    cpu = 0;
    advance(10_000);
    await waiting;
  });
});

describe("runHeavy", () => {
  test("one argument is a shell line; its exit is passed on and the command stops counting", async () => {
    const h = heavy();
    expect(await runHeavy(h, [`test -n "$${INSIDE}" && exit 3`], options())).toBe(3);
    expect(h.status().running).toEqual([]);
  });

  test("several arguments run the program directly", async () => {
    expect(await runHeavy(heavy(), ["sh", "-c", "exit 4"], options())).toBe(4);
  });

  test("a program that cannot start exits 127, says why on stderr, and stops counting", async () => {
    const h = heavy();
    const stderr = vi.spyOn(process.stderr, "write").mockImplementation(() => true);
    expect(await runHeavy(h, ["heavy-cmd-no-such-program", "x"], options())).toBe(127);
    expect(stderr.mock.calls).toEqual([["heavy-cmd: cannot run heavy-cmd-no-such-program: spawn heavy-cmd-no-such-program ENOENT\n"]]);
    expect(h.status().running).toEqual([]);
  });

  test("inside another heavy command it runs at once: no gap, no wait, over the CPU target", async () => {
    const h = heavy();
    cpu = 100;
    await claim(h, 101);
    process.env[INSIDE] = "1";
    expect(await runHeavy(h, ["exit 0"], options())).toBe(0);
    expect(pids(h.status())).toEqual({ running: [101], waiting: [] });
  });

  test("a command that could not start in time is an error, and never runs", async () => {
    const h = heavy();
    await claim(h, 101);
    await expect(runHeavy(h, [`touch ${path.join(dir, "ran")}`], options())).rejects.toBeInstanceOf(HeavyBusyError);
    expect(pids(h.status())).toEqual({ running: [101], waiting: [] });
    expect(existsSync(path.join(dir, "ran"))).toBe(false);
  });
});

/** The runs `dir`'s record keeps, the last ended first. */
const runs = (limit = 50): HeavyRun[] => readRuns(dir, { limit });
const at = (ms: number) => new Date(ms).toISOString();

describe("the record", () => {
  test("a command is recorded once it is over: every field, its exit code, its wait and its run", async () => {
    const h = heavy();
    h.release(await claim(h, 101));
    advance(10_000);
    const running = runHeavy(h, ["exit 3"], options({ label: "suite", waitSec: 60, cwd: "/work/api" }));
    expect(await running).toBe(3);
    expect(runs()).toEqual([
      {
        id: 1,
        project: "api",
        label: "suite",
        command: ["exit 3"],
        cwd: "/work/api",
        host: hostname(),
        pid: process.pid,
        agent: "task-73",
        heavy_id: 2,
        inside: null,
        queued_at: at(clock),
        started_at: at(clock),
        ended_at: at(clock),
        exit: 3,
        outcome: "ran",
      },
    ]);
    // Several arguments are kept as argv.
    advance(10_000);
    expect(await runHeavy(h, ["sh", "-c", "exit 0"], options())).toBe(0);
    expect(runs(1)[0]).toMatchObject({ command: ["sh", "-c", "exit 0"], cwd: process.cwd(), exit: 0 });
  });

  test("a wait that gives up is recorded with 75, never started", async () => {
    const h = heavy();
    await claim(h, 101);
    advance(5_000);
    await expect(runHeavy(h, ["exit 0"], options({ label: "npm test" }))).rejects.toBeInstanceOf(HeavyBusyError);
    expect(runs()).toMatchObject([{ label: "npm test", pid: process.pid, queued_at: at(clock), started_at: null, exit: 75, outcome: "gave up" }]);
  });

  test("a wait stopped by a signal is recorded with 128 + the signal; another abort, with no exit", async () => {
    const h = heavy();
    cpu = 100;
    await claim(h, 101);
    for (const reason of ["SIGTERM", new Error("stopped")]) {
      const stop = new AbortController();
      const waiting = heavily(h, options({ waitSec: 60, signal: stop.signal }), async () => "ran");
      await tick();
      advance(3_000);
      stop.abort(reason);
      await expect(waiting).rejects.toBeDefined();
    }
    expect(runs().map((run) => [run.outcome, run.exit, run.started_at])).toEqual([
      ["stopped while waiting", null, null],
      ["stopped while waiting", 143, null],
    ]);
    expect(pids(h.status())).toEqual({ running: [101], waiting: [] });
  });

  test("a command inside another is recorded as run inside that one's id, started at once", async () => {
    const h = heavy();
    process.env[INSIDE] = "7";
    expect(await runHeavy(h, ["exit 0"], options({ label: "inner" }))).toBe(0);
    expect(runs()).toMatchObject([{ label: "inner", inside: 7, queued_at: at(clock), started_at: at(clock), exit: 0, outcome: "ran" }]);
  });

  test("work that is not a command is recorded with exit 0, or 1 when it throws, and no command", async () => {
    const h = heavy();
    await heavily(h, options({ label: "warm cache" }), async () => "built");
    advance(10_000);
    await expect(heavily(h, options({ label: "warm cache" }), async () => Promise.reject(new Error("no image")))).rejects.toThrow("no image");
    expect(runs().map((run) => [run.label, run.command, run.exit])).toEqual([
      ["warm cache", null, 1],
      ["warm cache", null, 0],
    ]);
  });

  test("no agent is recorded as none, and the command sees the agent it ran for", async () => {
    const h = heavy();
    const out = path.join(dir, "agent");
    expect(await runHeavy(h, [`printf %s "$HEAVY_CMD_AGENT" > ${out}`], options({ agent: "task-9" }))).toBe(0);
    advance(10_000);
    await runHeavy(h, ["exit 0"], options({ agent: null, label: "anonymous" }));
    expect(runs().map((run) => [run.label, run.agent])).toEqual([
      ["anonymous", null],
      ["npm test", "task-9"],
    ]);
    expect(readFileSync(out, "utf8")).toBe("task-9");
  });

  test("a new run forgets those that ended over 30 days before it", async () => {
    const h = heavy();
    const ran = (label: string) => h.record(options({ label }), { queued_at: at(clock), started_at: at(clock), exit: 0, outcome: "ran" });
    ran("old");
    advance(30 * 86_400_000);
    ran("a month");
    advance(1);
    ran("new");
    expect(runs().map((run) => run.label)).toEqual(["new", "a month"]);
  });

  test("a record that cannot be written leaves the exit code as it is and says one line on stderr", async () => {
    const h = heavy();
    const db = new DatabaseSync(path.join(dir, "heavy.db"));
    db.exec("CREATE TRIGGER full BEFORE INSERT ON runs BEGIN SELECT RAISE(FAIL, 'database or disk is full'); END");
    db.close();
    const stderr = vi.spyOn(process.stderr, "write").mockImplementation(() => true);
    expect(await runHeavy(h, ["exit 5"], options())).toBe(5);
    expect(stderr.mock.calls).toEqual([["heavy-cmd: the heavy command's run was not recorded: database or disk is full\n"]]);
    expect(h.status().running).toEqual([]);
  });
});

describe("heavy-cmd logs", () => {
  /** What `heavy-cmd logs` prints with `flags`, its `HEAVY_CMD_HOME` the queue's folder. */
  async function log(flags: Record<string, string[]> = {}): Promise<string> {
    const stdout = vi.spyOn(process.stdout, "write").mockImplementation(() => true);
    try {
      cmdLogs(flags, { HEAVY_CMD_HOME: dir });
      return stdout.mock.calls.map(([text]) => String(text)).join("");
    } finally {
      stdout.mockRestore();
    }
  }

  afterEach(() => {
    vi.useRealTimers();
  });

  test("lists the machine's runs, the last ended first, one line each; --json gives every field", async () => {
    // The log dates a run ended on another day than now's: at noon, every run below ended today.
    vi.setSystemTime(new Date().setHours(12, 0, 0, 0));
    expect(await log()).toBe("no heavy command recorded\n");
    const h = heavy();
    clock = Date.now() - 2 * 3_600_000;
    await runHeavy(h, ["exit 0"], options({ label: "npm test" }));
    clock = Date.now() - 600_000;
    h.release(await claim(h, 101, { project: "web" }));
    advance(1_000);
    const running = runHeavy(h, ["exit 0"], options({ project: "web", label: "npm run check", waitSec: 60 }));
    await tick();
    advance(11_000);
    expect(await running).toBe(0);
    advance(3_000);
    await expect(runHeavy(h, ["exit 0"], options({ project: "web", label: "npm run build" }))).rejects.toBeInstanceOf(HeavyBusyError);
    const lines = (await log()).trimEnd().split("\n");
    const time = (ms: number) => new Date(ms).toTimeString().slice(0, 8);
    expect(lines[0]).toBe(`${time(clock)}  web  npm run build  gave up after 0s (75)  task-73`);
    expect(lines[1]).toBe(`${time(clock - 3_000)}  web  npm run check  exit 0  waited 11s  ran 0s  task-73`);
    expect(lines[2]).toMatch(/ {2}api {2}npm test {2}exit 0 {2}waited 0s {2}ran 0s {2}task-73$/);
    const json = JSON.parse(await log({ json: [""] })) as HeavyRun[];
    expect(json.map((run) => run.label)).toEqual(["npm run build", "npm run check", "npm test"]);
    expect(Object.keys(json[0]!).sort()).toEqual(
      ["agent", "command", "cwd", "ended_at", "exit", "heavy_id", "host", "id", "inside", "label", "outcome", "pid", "project", "queued_at", "started_at"].sort(),
    );
  });

  test("--project, --limit and --since, an ISO time or a duration, filter it", async () => {
    const h = heavy();
    const ran = (project: string, label: string, ago: number) => {
      clock = Date.now() - ago;
      h.record({ project, label }, { queued_at: at(clock), started_at: at(clock), exit: 0, outcome: "ran" });
    };
    ran("api", "two hours", 2 * 3_600_000);
    ran("web", "half an hour", 1_800_000);
    ran("api", "a minute", 60_000);
    const labels = async (flags: Record<string, string[]>) => (JSON.parse(await log({ ...flags, json: [""] })) as HeavyRun[]).map((run) => run.label);
    expect(await labels({ project: ["api"] })).toEqual(["a minute", "two hours"]);
    expect(await labels({ limit: ["2"] })).toEqual(["a minute", "half an hour"]);
    expect(await labels({ since: ["1h"] })).toEqual(["a minute", "half an hour"]);
    expect(await labels({ since: ["45m"], project: ["web"] })).toEqual(["half an hour"]);
    expect(await labels({ since: [at(Date.now() - 1_000_000)] })).toEqual(["a minute"]);
    await expect(log({ since: ["yesterday"] })).rejects.toThrow('--since takes an ISO time or a duration such as 1h, not "yesterday"');
    await expect(log({ limit: ["-1"] })).rejects.toThrow("--limit must be a non-negative integer");
  });
});

test("a run's line on another day carries its date; a stopped wait with no exit says none", () => {
  const run: HeavyRun = {
    id: 1,
    project: "api",
    label: "npm ci",
    command: ["npm ci"],
    cwd: "/",
    host: "h",
    pid: 1,
    agent: null,
    heavy_id: null,
    inside: 4,
    queued_at: "2026-01-02T10:00:00.000Z",
    started_at: null,
    ended_at: "2026-01-02T10:00:42.000Z",
    exit: null,
    outcome: "stopped while waiting",
  };
  const ended = new Date(run.ended_at);
  const date = `${ended.toLocaleString("en-US", { month: "short" })} ${ended.getDate()} ${ended.toTimeString().slice(0, 8)}`;
  expect(runLine(run, Date.parse("2026-03-01T12:00:00Z"))).toBe(`${date}  api  npm ci  stopped while waiting after 42s  inside heavy 4`);
});
