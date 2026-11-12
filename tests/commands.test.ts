import { afterEach, beforeEach, describe, expect, test, vi } from "vitest";
import { execFileSync } from "node:child_process";
import { mkdirSync, mkdtempSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import path from "node:path";
import { cmdRun, cmdStatus } from "../src/commands.ts";
import { readRuns } from "../src/queue.ts";

let dir: string;
let env: NodeJS.ProcessEnv;

beforeEach(() => {
  dir = mkdtempSync(path.join(tmpdir(), "heavy-cmd-commands-"));
  env = { ...process.env, HEAVY_CMD_HOME: path.join(dir, "state"), XDG_CONFIG_HOME: path.join(dir, "config") };
});

afterEach(() => {
  vi.restoreAllMocks();
  rmSync(dir, { recursive: true, force: true });
});

/** What `use` writes on stdout. */
async function stdoutOf(use: () => Promise<unknown>): Promise<string> {
  const stdout = vi.spyOn(process.stdout, "write").mockImplementation(() => true);
  try {
    await use();
    return stdout.mock.calls.map(([text]) => String(text)).join("");
  } finally {
    stdout.mockRestore();
  }
}

describe("heavy-cmd -- <command>", () => {
  test("is recorded under --project, --agent and --label when given", async () => {
    expect(await cmdRun(["exit 2"], { project: ["web"], agent: ["task-9"], label: ["the suite"] }, { ...env, HEAVY_CMD_AGENT: "other" }, dir)).toBe(2);
    expect(readRuns(env.HEAVY_CMD_HOME!, { limit: 5 })).toMatchObject([{ project: "web", agent: "task-9", label: "the suite", cwd: dir, exit: 2, command: ["exit 2"] }]);
  });

  test("else under the cwd's project, HEAVY_CMD_AGENT, and the command's first 80 characters", async () => {
    const repo = path.join(dir, "worktree-folder");
    mkdirSync(repo);
    execFileSync("git", ["-C", repo, "init", "-q"]);
    execFileSync("git", ["-C", repo, "remote", "add", "origin", "https://github.com/acme/api.git"]);
    const long = `true ${"x".repeat(100)}`;
    expect(await cmdRun([long], {}, { ...env, HEAVY_CMD_AGENT: "task-73" }, repo)).toBe(0);
    expect(readRuns(env.HEAVY_CMD_HOME!, { limit: 5 })).toMatchObject([{ project: "api", agent: "task-73", label: long.slice(0, 80), cwd: repo }]);
  });

  test("with no agent named, none is recorded", async () => {
    await cmdRun(["true"], {}, env, dir);
    expect(readRuns(env.HEAVY_CMD_HOME!, { limit: 5 })[0]!.agent).toBeNull();
  });

  test("a setting out of bounds is refused before anything runs", async () => {
    await expect(cmdRun(["true"], { wait: ["90000"] }, env, dir)).rejects.toThrow("--wait must be an integer number of seconds between 0 and 86400");
    expect(readRuns(env.HEAVY_CMD_HOME!, { limit: 5 })).toEqual([]);
  });
});

describe("heavy-cmd status", () => {
  test("--json has the fields of factory heavy status --json, the target from the config", async () => {
    await cmdRun(["true"], {}, env, dir);
    const status = JSON.parse(await stdoutOf(() => cmdStatus({ json: [""] }, { ...env, HEAVY_CMD_CPU: "70" }))) as Record<string, unknown>;
    expect(Object.keys(status).sort()).toEqual(["cpu", "last_start", "running", "since_last_start_sec", "target", "waiting"]);
    expect(status).toMatchObject({ running: [], waiting: [], target: 70 });
    expect(status.since_last_start_sec).toBeGreaterThanOrEqual(0);
  });

  test("the human view starts with the summary", async () => {
    expect(await stdoutOf(() => cmdStatus({}, env))).toBe("0 running · cpu ?/60%, no start yet\n");
  });
});
