import { afterEach, beforeEach, describe, expect, test } from "vitest";
import { execFileSync, type spawn } from "node:child_process";
import { copyFileSync, existsSync, mkdirSync, mkdtempSync, readdirSync, readlinkSync, rmSync, symlinkSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import path from "node:path";
import { compareVersions, installOf, NODE_FLAGS, ROOT } from "../src/install.ts";
import { autoUpdate, githubPorts, update, type UpdatePorts } from "../src/update.ts";
import { release as releaseIn, serveReleases } from "./releases.ts";

let dir: string;
/** The install `update` moves: `<share>/versions/<tag>` and `<share>/current`. */
let share: string;
let said: string[];
let downloads: string[];

beforeEach(() => {
  dir = mkdtempSync(path.join(tmpdir(), "heavy-cmd-update-"));
  share = path.join(dir, "share");
  said = [];
  downloads = [];
});

afterEach(() => rmSync(dir, { recursive: true, force: true }));

const release = (tag: string, bin?: string) => releaseIn(dir, tag, bin);

/** An install made by install.sh, at `tag`: what runs there is the folder returned. */
function installed(tag: string): string {
  const root = path.join(share, "versions", tag);
  mkdirSync(root, { recursive: true });
  execFileSync("tar", ["-xzf", release(tag), "-C", root, "--strip-components=1"]);
  symlinkSync(path.join("versions", tag), path.join(share, "current"));
  return root;
}

/** The releases: the newest is `tag`, its tarball `tarball`. */
const ports = (tag: string, tarball = release(tag)): UpdatePorts => ({
  latest: async () => ({ tag, tarball }),
  download: async (url, file) => {
    downloads.push(url);
    copyFileSync(url, file);
  },
});

const say = (line: string) => void said.push(line);
const current = () => readlinkSync(path.join(share, "current"));
const runs = () => execFileSync(process.execPath, [...NODE_FLAGS, path.join(share, "current", "bin", "heavy-cmd"), "--version"], { encoding: "utf8" }).trim();

describe("heavy-cmd update", () => {
  test("refuses in a dev checkout, and anywhere install.sh did not install", async () => {
    expect(installOf(ROOT).kind).toBe("dev");
    await expect(update(ports("v9.0.0", "unused"), false, ROOT, say)).rejects.toThrow(`${ROOT} is a dev checkout: update it with git`);
    await expect(update(ports("v9.0.0", "unused"), false, dir, say)).rejects.toThrow(`${dir} is not an install made by install.sh`);
  });

  test("moves the install to the newest release, keeping the one before it, and drops older ones", async () => {
    const root = installed("v0.1.0");
    await update(ports("v0.2.0"), false, root, say);
    expect(current()).toBe(path.join("versions", "v0.2.0"));
    expect(runs()).toBe("heavy-cmd v0.2.0");
    await update(ports("v0.3.0"), false, root, say);
    expect(runs()).toBe("heavy-cmd v0.3.0");
    expect(readdirSync(path.join(share, "versions")).sort()).toEqual(["v0.2.0", "v0.3.0"]);
    expect(said).toEqual(["heavy-cmd updated from v0.1.0 to v0.2.0", "heavy-cmd updated from v0.2.0 to v0.3.0"]);
  });

  test("never moves back, nor downloads what it has", async () => {
    const root = installed("v0.2.0");
    await update(ports("v0.1.9"), false, root, say);
    await update(ports("v0.2.0"), false, root, say);
    expect(downloads).toEqual([]);
    expect(current()).toBe(path.join("versions", "v0.2.0"));
    expect(said).toEqual(["heavy-cmd v0.2.0 is up to date (the newest release is v0.1.9)", "heavy-cmd v0.2.0 is up to date (the newest release is v0.2.0)"]);
  });

  test("--check only says whether a newer one is out", async () => {
    const root = installed("v0.1.0");
    await update(ports("v0.10.0"), true, root, say);
    expect(said).toEqual(["heavy-cmd v0.10.0 is out; this is v0.1.0: heavy-cmd update installs it"]);
    expect(downloads).toEqual([]);
    expect(current()).toBe(path.join("versions", "v0.1.0"));
  });

  test("a new version that does not run, or names another, is not switched to; nothing of it is left", async () => {
    const root = installed("v0.1.0");
    const broken = release("v0.2.0", "#!/bin/sh\nexit 1\n");
    await expect(update(ports("v0.2.0", broken), false, root, say)).rejects.toThrow(/update to v0\.2\.0 failed, v0\.1\.0 kept: v0\.2\.0 does not run here/);
    await expect(update(ports("v0.3.0", release("v0.2.5")), false, root, say)).rejects.toThrow('its --version said "heavy-cmd v0.2.5"');
    await expect(update({ ...ports("v0.4.0", "unused"), download: async () => Promise.reject(new Error("offline")) }, false, root, say)).rejects.toThrow(
      "update to v0.4.0 failed, v0.1.0 kept: offline",
    );
    expect(current()).toBe(path.join("versions", "v0.1.0"));
    expect(readdirSync(path.join(share, "versions"))).toEqual(["v0.1.0"]);
    expect(existsSync(path.join(share, "update.lock"))).toBe(false);
    expect(runs()).toBe("heavy-cmd v0.1.0");
  });

  test("one update at a time: a live one's lock refuses another; a dead one's is taken over", async () => {
    const root = installed("v0.1.0");
    writeFileSync(path.join(share, "update.lock"), String(process.pid));
    await expect(update(ports("v0.2.0"), false, root, say)).rejects.toThrow(`another heavy-cmd update is running (pid ${process.pid})`);
    writeFileSync(path.join(share, "update.lock"), "999999999");
    await update(ports("v0.2.0"), false, root, say);
    expect(runs()).toBe("heavy-cmd v0.2.0");
  });

  test("reads the newest release and downloads its tarball over HTTP, as from GitHub's API", async () => {
    const releases = await serveReleases();
    try {
      releases.newest = { tag: "v0.2.0", tarball: release("v0.2.0") };
      const root = installed("v0.1.0");
      await update(githubPorts({ HEAVY_CMD_RELEASES_API: releases.api }), false, root, say);
      expect(runs()).toBe("heavy-cmd v0.2.0");
      await expect(githubPorts({ HEAVY_CMD_RELEASES_API: `${releases.base}/no-asset` }).latest()).rejects.toThrow("release v0.2.0 has no heavy-cmd.tar.gz");
      await expect(githubPorts({ HEAVY_CMD_RELEASES_API: `${releases.base}/gone` }).latest()).rejects.toThrow(`${releases.base}/gone: 404`);
      await expect(githubPorts({ HEAVY_CMD_RELEASES_API: "http://127.0.0.1:1/latest" }).latest()).rejects.toThrow(/^http:\/\/127\.0\.0\.1:1\/latest: \w/);
    } finally {
      releases.close();
    }
  });

  test("versions compare by number, not as text", () => {
    expect(compareVersions("v0.10.0", "v0.9.9")).toBeGreaterThan(0);
    expect(compareVersions("v1.0.0", "1.0.0")).toBe(0);
    expect(compareVersions("v1.2.3", "v1.3.0")).toBeLessThan(0);
    expect(() => compareVersions("v1", "v1.0.0")).toThrow("not a release version: v1");
  });
});

describe("the auto update", () => {
  const DAY = 86_400_000;
  let started: unknown[][];
  let clock: number;
  const spawned = ((...args: unknown[]) => {
    started.push(args);
    return { on: () => undefined, unref: () => undefined };
  }) as unknown as typeof spawn;
  const auto = (root: string, env: NodeJS.ProcessEnv = { HEAVY_CMD_AUTO_UPDATE: "on" }) => autoUpdate({ ...process.env, ...env }, root, { now: () => clock, spawn: spawned });

  beforeEach(() => {
    started = [];
    clock = Date.parse("2026-11-12T10:00:00Z");
  });

  test("starts heavy-cmd update detached, its output in update.log, at most once a day", () => {
    const root = installed("v0.1.0");
    expect(auto(root)).toBe(true);
    expect(started).toHaveLength(1);
    const [file, args, options] = started[0] as [string, string[], { detached: boolean; stdio: unknown[] }];
    expect(file).toBe(process.execPath);
    expect(args).toEqual([...NODE_FLAGS, path.join(share, "current", "bin", "heavy-cmd"), "update"]);
    expect(options.detached).toBe(true);
    expect(options.stdio[0]).toBe("ignore");
    expect(existsSync(path.join(share, "update.log"))).toBe(true);
    clock += DAY - 1;
    expect(auto(root)).toBe(false);
    clock += 1;
    expect(auto(root)).toBe(true);
    expect(started).toHaveLength(2);
  });

  test("never with auto_update = off, never in a dev checkout or elsewhere", () => {
    const root = installed("v0.1.0");
    expect(auto(root, { HEAVY_CMD_AUTO_UPDATE: "off" })).toBe(false);
    const config = path.join(dir, "config");
    mkdirSync(path.join(config, "heavy-cmd"), { recursive: true });
    writeFileSync(path.join(config, "heavy-cmd", "config"), "auto_update = off\n");
    expect(auto(root, { HEAVY_CMD_AUTO_UPDATE: "", XDG_CONFIG_HOME: config })).toBe(false);
    expect(auto(ROOT)).toBe(false);
    expect(auto(dir)).toBe(false);
    expect(started).toEqual([]);
  });

  test("whatever fails is ignored", () => {
    const root = installed("v0.1.0");
    const failing = (() => {
      throw new Error("EAGAIN");
    }) as unknown as typeof spawn;
    expect(autoUpdate({ ...process.env, HEAVY_CMD_AUTO_UPDATE: "on" }, root, { now: () => clock, spawn: failing })).toBe(false);
    expect(auto(root, { HEAVY_CMD_AUTO_UPDATE: "maybe" })).toBe(false);
  });
});
