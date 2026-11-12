import { afterEach, beforeEach, describe, expect, test } from "vitest";
import { spawn, spawnSync, type ChildProcess } from "node:child_process";
import { mkdtempSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import path from "node:path";
import { ROOT, versionOf } from "../src/install.ts";
import type { HeavyRun } from "../src/runs.ts";

const BIN = path.join(ROOT, "bin", "heavy-cmd");
/** A command that holds its start until the test ends. */
const HOLDER = ["--label", "holder", "--", "echo started; exec sleep 30"];

let dir: string;
let env: NodeJS.ProcessEnv;
const started: ChildProcess[] = [];

beforeEach(() => {
  dir = mkdtempSync(path.join(tmpdir(), "heavy-cmd-cli-"));
  env = { ...process.env, HEAVY_CMD_HOME: path.join(dir, "state"), XDG_CONFIG_HOME: path.join(dir, "config") };
});

afterEach(async () => {
  // SIGTERM reaches the command through heavy-cmd, which then ends.
  await Promise.all(started.splice(0).map((child) => (child.exitCode !== null || child.signalCode !== null ? null : (child.kill("SIGTERM"), ended(child)))));
  rmSync(dir, { recursive: true, force: true });
});

/** `heavy-cmd <args>`, run to its end. */
function cli(args: string[], extra: NodeJS.ProcessEnv = {}) {
  const result = spawnSync(BIN, args, { cwd: dir, env: { ...env, ...extra }, encoding: "utf8", timeout: 60_000 });
  return { code: result.status, signal: result.signal, stdout: result.stdout, stderr: result.stderr };
}

/** `heavy-cmd <args>` left running, resolved once its stderr or stdout says `ready`. */
function background(args: string[], ready: RegExp): Promise<ChildProcess> {
  const child = spawn(BIN, args, { cwd: dir, env, stdio: ["ignore", "pipe", "pipe"] });
  started.push(child);
  let seen = "";
  return new Promise((resolve, reject) => {
    const look = (chunk: Buffer) => {
      seen += chunk.toString();
      if (ready.test(seen)) resolve(child);
    };
    child.stdout!.on("data", look);
    child.stderr!.on("data", look);
    child.on("exit", (code) => reject(new Error(`heavy-cmd ${args.join(" ")} exited ${code} before ${ready}: ${seen}`)));
  });
}

const ended = (child: ChildProcess) => new Promise<{ code: number | null; signal: NodeJS.Signals | null }>((resolve) => child.on("exit", (code, signal) => resolve({ code, signal })));
const logs = (): HeavyRun[] => JSON.parse(cli(["logs", "--json"]).stdout) as HeavyRun[];

describe("the command line", () => {
  test("--version names the version, and a dev checkout", () => {
    expect(cli(["--version"])).toMatchObject({ code: 0, stdout: `heavy-cmd v${versionOf()} (dev checkout)\n` });
  });

  test("runs the command in the foreground: its output, its exit code; one argument is a shell line", () => {
    // Each in a home of its own: a second start in one home waits out the gap.
    const home = (name: string) => ({ HEAVY_CMD_HOME: path.join(dir, name) });
    expect(cli(["--", "sh", "-c", "echo out; echo err >&2; exit 3"], home("a"))).toMatchObject({ code: 3, stdout: "out\n", stderr: "err\n" });
    expect(cli(["--", 'test -n "$HEAVY_CMD_ID" && echo "$HEAVY_CMD_ID"'], home("b"))).toMatchObject({ code: 0, stdout: "1\n" });
    expect(cli(["--", "heavy-cmd-no-such-program"], home("c"))).toMatchObject({ code: 127 });
  });

  test("a command line it does not take exits 64; a setting out of bounds exits 78, naming why", () => {
    expect(cli([])).toMatchObject({ code: 64 });
    expect(cli(["npm", "test"])).toMatchObject({ code: 64, stderr: 'heavy-cmd: unknown command "npm": a command to run goes after --, as in heavy-cmd -- npm test\n' });
    expect(cli(["--wiat", "3", "--", "true"])).toMatchObject({ code: 64, stderr: "heavy-cmd: unknown option --wiat\n" });
    expect(cli(["--"])).toMatchObject({ code: 64, stderr: "heavy-cmd: nothing to run after --\n" });
    expect(cli(["status", "--wait", "3"])).toMatchObject({ code: 64, stderr: "heavy-cmd: status takes no --wait\n" });
    expect(cli(["logs", "--since", "yesterday"])).toMatchObject({ code: 64 });
    expect(cli(["--", "true"], { HEAVY_CMD_CPU: "5" })).toMatchObject({ code: 78, stderr: 'heavy-cmd: HEAVY_CMD_CPU must be an integer between 10 and 100, not "5"\n' });
    expect(cli(["update"])).toMatchObject({ code: 1, stderr: expect.stringContaining("is a dev checkout: update it with git") });
    expect(logs()).toEqual([]);
  });

  test("one that cannot start within --wait says it waits, then exits 75, recorded as given up", async () => {
    await background(HOLDER, /started/);
    await waitFor(() => JSON.parse(cli(["status", "--json"]).stdout).running.length === 1);
    const result = cli(["--wait", "3", "--label", "late", "--", "true"]);
    expect(result.code).toBe(75);
    expect(result.stderr).toMatch(/^heavy-cmd: every heavy slot is busy: 1 running · cpu (\?|\d+%)\/60%, last start \d+s ago \(heavy-cmd-cli-\w+: holder\); waiting up to 3s\n/);
    expect(result.stderr).toMatch(/heavy-cmd: no heavy command could start in 3s\n$/);
    expect(logs()).toMatchObject([{ label: "late", exit: 75, outcome: "gave up", started_at: null }]);
  }, 30_000);

  test("inside another heavy-cmd it runs at once, recorded as inside that one", async () => {
    await background(HOLDER, /started/);
    await waitFor(() => JSON.parse(cli(["status", "--json"]).stdout).running.length === 1);
    expect(cli(["--wait", "0", "--agent", "task-1", "--", "true"], { HEAVY_CMD_ID: "1" })).toMatchObject({ code: 0 });
    expect(logs()).toMatchObject([{ inside: 1, agent: "task-1", outcome: "ran", exit: 0 }]);
  }, 30_000);

  test("a signal while it waits stops the wait, is recorded, and ends heavy-cmd as it would the command", async () => {
    await background(HOLDER, /started/);
    await waitFor(() => JSON.parse(cli(["status", "--json"]).stdout).running.length === 1);
    const waiter = await background(["--label", "waiter", "--", "true"], /every heavy slot is busy/);
    const end = ended(waiter);
    waiter.kill("SIGTERM");
    expect(await end).toEqual({ code: null, signal: "SIGTERM" });
    expect(logs()).toMatchObject([{ label: "waiter", exit: 143, outcome: "stopped while waiting", started_at: null }]);
  }, 30_000);

  test("smoke: a second command waits out the gap after the first; logs shows both", () => {
    expect(cli(["--label", "first", "--agent", "smoke", "--", "true"])).toMatchObject({ code: 0, stderr: "" });
    const before = Date.now();
    const second = cli(["--label", "second", "--agent", "smoke", "--", "true"]);
    expect(Date.now() - before).toBeGreaterThanOrEqual(8_000);
    expect(second.code).toBe(0);
    expect(second.stderr).toMatch(/^heavy-cmd: every heavy slot is busy: 0 running/);
    const lines = cli(["logs"]).stdout.trimEnd().split("\n");
    expect(lines).toHaveLength(2);
    expect(lines[0]).toMatch(/ {2}heavy-cmd-cli-\w+ {2}second {2}exit 0 {2}waited (9|1\d)s {2}ran 0s {2}smoke$/);
    expect(lines[1]).toMatch(/ {2}heavy-cmd-cli-\w+ {2}first {2}exit 0 {2}waited 0s {2}ran 0s {2}smoke$/);
  }, 40_000);
});

/** Polls `ok` until it holds, for up to 10 seconds. */
async function waitFor(ok: () => boolean): Promise<void> {
  for (let tries = 0; tries < 100; tries++) {
    if (ok()) return;
    await new Promise((resolve) => setTimeout(resolve, 100));
  }
  throw new Error("timed out");
}
