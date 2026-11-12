import { afterEach, beforeEach, describe, expect, test } from "vitest";
import { spawn } from "node:child_process";
import { mkdirSync, mkdtempSync, readdirSync, readlinkSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import path from "node:path";
import { ROOT } from "../src/install.ts";
import { release, serveReleases } from "./releases.ts";

let dir: string;
let releases: Awaited<ReturnType<typeof serveReleases>>;
let env: NodeJS.ProcessEnv;

beforeEach(async () => {
  dir = mkdtempSync(path.join(tmpdir(), "heavy-cmd-install-"));
  releases = await serveReleases();
  env = {
    ...process.env,
    HEAVY_CMD_RELEASES_API: releases.api,
    HEAVY_CMD_INSTALL_DIR: path.join(dir, "share"),
    HEAVY_CMD_BIN_DIR: path.join(dir, "bin"),
    HEAVY_CMD_HOME: path.join(dir, "state"),
  };
});

afterEach(() => {
  releases.close();
  rmSync(dir, { recursive: true, force: true });
});

/** `file args` run to its end, without blocking the release server in this process. */
function run(file: string, args: string[] = []): Promise<{ code: number | null; stdout: string; stderr: string }> {
  const child = spawn(file, args, { env, stdio: ["ignore", "pipe", "pipe"] });
  let stdout = "";
  let stderr = "";
  child.stdout.on("data", (chunk: Buffer) => (stdout += chunk.toString()));
  child.stderr.on("data", (chunk: Buffer) => (stderr += chunk.toString()));
  return new Promise((resolve) => child.on("close", (code) => resolve({ code, stdout, stderr })));
}
const installSh = () => run("sh", [path.join(ROOT, "install.sh")]);
const heavyCmd = (...args: string[]) => run(path.join(dir, "bin", "heavy-cmd"), args);

describe("install.sh", () => {
  test("installs the newest release into the two folders, and again changes nothing", async () => {
    releases.newest = { tag: "v0.2.0", tarball: release(dir, "v0.2.0") };
    expect(await installSh()).toMatchObject({ code: 0, stdout: `heavy-cmd v0.2.0 installed: ${path.join(dir, "bin", "heavy-cmd")}\n` });
    expect(readlinkSync(path.join(dir, "bin", "heavy-cmd"))).toBe(path.join(dir, "share", "current", "bin", "heavy-cmd"));
    expect(readlinkSync(path.join(dir, "share", "current"))).toBe(path.join("versions", "v0.2.0"));
    expect(await heavyCmd("--version")).toMatchObject({ code: 0, stdout: "heavy-cmd v0.2.0\n" });
    const again = await installSh();
    expect(again).toMatchObject({ code: 0, stderr: expect.stringContaining("v0.2.0 is already installed") });
    expect(readdirSync(path.join(dir, "share", "versions"))).toEqual(["v0.2.0"]);
    expect(await heavyCmd("--", "true")).toMatchObject({ code: 0 });
  }, 60_000);

  test("what it installs updates itself: update --check, then update", async () => {
    releases.newest = { tag: "v0.2.0", tarball: release(dir, "v0.2.0") };
    await installSh();
    expect(await heavyCmd("update", "--check")).toMatchObject({ code: 0, stdout: "heavy-cmd v0.2.0 is up to date (the newest release is v0.2.0)\n" });
    releases.newest = { tag: "v0.3.0", tarball: release(dir, "v0.3.0") };
    expect(await heavyCmd("update", "--check")).toMatchObject({ code: 0, stdout: "heavy-cmd v0.3.0 is out; this is v0.2.0: heavy-cmd update installs it\n" });
    expect(await heavyCmd("update")).toMatchObject({ code: 0, stdout: "heavy-cmd updated from v0.2.0 to v0.3.0\n" });
    expect(await heavyCmd("--version")).toMatchObject({ stdout: "heavy-cmd v0.3.0\n" });
  }, 60_000);

  test("a release that does not run is not installed; a file in the link's place is left alone", async () => {
    releases.newest = { tag: "v0.2.0", tarball: release(dir, "v0.2.0", "#!/bin/sh\nexit 1\n") };
    expect(await installSh()).toMatchObject({ code: 1, stderr: expect.stringContaining("v0.2.0 does not run here") });
    expect(readdirSync(path.join(dir, "share", "versions"))).toEqual([]);
    releases.newest = { tag: "v0.2.0", tarball: release(dir, "v0.2.0") };
    mkdirSync(path.join(dir, "bin"), { recursive: true });
    writeFileSync(path.join(dir, "bin", "heavy-cmd"), "mine");
    expect(await installSh()).toMatchObject({ code: 1, stderr: expect.stringContaining("exists and is not a link") });
  }, 60_000);
});
