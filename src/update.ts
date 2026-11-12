import { execFileSync, spawn } from "node:child_process";
import { closeSync, mkdirSync, mkdtempSync, openSync, readdirSync, readFileSync, renameSync, rmSync, symlinkSync, writeFileSync } from "node:fs";
import path from "node:path";
import { loadConfig } from "./config.ts";
import { pidAlive } from "./format.ts";
import { ASSET, compareVersions, currentTag, installOf, NODE_FLAGS, parseVersion, ROOT, versionOf } from "./install.ts";

/** Where the newest release is read from; `HEAVY_CMD_RELEASES_API` points elsewhere, a mirror or a test's server. */
export const RELEASES_API = "https://api.github.com/repos/Ploppy3/heavy-cmd/releases/latest";

/** A release by its tag, `v0.2.0`, and the URL of its tarball. */
export type Release = { tag: string; tarball: string };

export type UpdatePorts = {
  latest: () => Promise<Release>;
  download: (url: string, file: string) => Promise<void>;
};

const HEADERS = { "user-agent": "heavy-cmd", accept: "application/vnd.github+json" };

async function fetched(url: string, timeoutMs: number): Promise<Response> {
  const response = await fetch(url, { headers: HEADERS, signal: AbortSignal.timeout(timeoutMs) }).catch((error: unknown) => {
    // fetch says only "fetch failed"; why is in its cause.
    const why = (error as { cause?: { message?: string } }).cause?.message ?? (error as Error).message;
    throw new Error(`${url}: ${why}`);
  });
  if (!response.ok) throw new Error(`${url}: ${response.status} ${response.statusText}`);
  return response;
}

/** The newest release, as GitHub's `releases/latest` gives it: its tag and its tarball asset. */
export async function latestRelease(url: string): Promise<Release> {
  const body = (await (await fetched(url, 30_000)).json()) as { tag_name?: string; assets?: { name: string; browser_download_url: string }[] };
  const tag = body.tag_name ?? "";
  if (!parseVersion(tag)) throw new Error(`${url}: no release tag, read "${tag}"`);
  const asset = body.assets?.find((each) => each.name === ASSET);
  if (!asset) throw new Error(`release ${tag} has no ${ASSET}`);
  return { tag, tarball: asset.browser_download_url };
}

/** The releases on GitHub, or the API `HEAVY_CMD_RELEASES_API` names. */
export const githubPorts = (env = process.env): UpdatePorts => ({
  latest: () => latestRelease(env.HEAVY_CMD_RELEASES_API || RELEASES_API),
  download: async (url, file) => writeFileSync(file, Buffer.from(await (await fetched(url, 120_000)).arrayBuffer())),
});

/** What `bin/heavy-cmd --version` of `root` prints, or why it did not run. */
function runVersion(root: string): string {
  try {
    const bin = path.join(root, "bin", "heavy-cmd");
    return execFileSync(process.execPath, [...NODE_FLAGS, bin, "--version"], { encoding: "utf8", timeout: 30_000, stdio: ["ignore", "pipe", "pipe"] }).trim();
  } catch (error) {
    const stderr = String((error as { stderr?: unknown }).stderr ?? "").trim();
    return `failed: ${stderr || (error as Error).message}`;
  }
}

/** `work` while this process holds `<dir>/update.lock`: two updates never switch at once. One left by a dead process is taken over. */
async function locked<T>(dir: string, work: () => Promise<T>): Promise<T> {
  const lock = path.join(dir, "update.lock");
  for (let tries = 0; ; tries++) {
    try {
      writeFileSync(lock, String(process.pid), { flag: "wx" });
      break;
    } catch (error) {
      if ((error as NodeJS.ErrnoException).code !== "EEXIST" || tries > 0) throw error;
      const pid = Number(readFileSync(lock, "utf8"));
      if (Number.isInteger(pid) && pid > 0 && pidAlive(pid)) throw new Error(`another heavy-cmd update is running (pid ${pid})`);
      rmSync(lock, { force: true });
    }
  }
  try {
    return await work();
  } finally {
    rmSync(lock, { force: true });
  }
}

/**
 * Unpacks `release` beside the versions in `dir`, and only once its own
 * `--version` names it switches `<dir>/current` to it, with one rename. The
 * versions but it and `previous` go. A failure leaves `current` as it was.
 */
async function install(dir: string, previous: string, release: Release, ports: UpdatePorts): Promise<void> {
  const versions = path.join(dir, "versions");
  const work = mkdtempSync(path.join(versions, `.${release.tag}-`));
  try {
    const tarball = path.join(work, ASSET);
    await ports.download(release.tarball, tarball);
    const unpacked = path.join(work, "heavy-cmd");
    mkdirSync(unpacked);
    execFileSync("tar", ["-xzf", tarball, "-C", unpacked, "--strip-components=1"], { stdio: ["ignore", "ignore", "pipe"] });
    const said = runVersion(unpacked);
    if (said !== `heavy-cmd ${release.tag}`) throw new Error(`${release.tag} does not run here: its --version said "${said}"`);
    const target = path.join(versions, release.tag);
    rmSync(target, { recursive: true, force: true });
    renameSync(unpacked, target);
    const link = path.join(dir, `.current-${process.pid}`);
    rmSync(link, { force: true });
    symlinkSync(path.join("versions", release.tag), link);
    renameSync(link, path.join(dir, "current"));
    for (const name of readdirSync(versions)) {
      if (!name.startsWith(".") && name !== release.tag && name !== previous) rmSync(path.join(versions, name), { recursive: true, force: true });
    }
  } finally {
    rmSync(work, { recursive: true, force: true });
  }
}

/**
 * `heavy-cmd update [--check]`: moves the install `root` belongs to to the
 * newest release, never back to an older one; `check` only says whether a
 * newer one is out. A dev checkout, or anything install.sh did not make, is
 * refused.
 */
export async function update(ports: UpdatePorts, check: boolean, root = ROOT, say = (line: string) => void process.stdout.write(`${line}\n`)): Promise<void> {
  const where = installOf(root);
  if (where.kind === "dev") throw new Error(`${root} is a dev checkout: update it with git; heavy-cmd update moves only an install made by install.sh`);
  if (where.kind === "other") throw new Error(`${root} is not an install made by install.sh: nothing to update`);
  const current = currentTag(where.dir) ?? `v${versionOf(root)}`;
  const release = await ports.latest();
  if (compareVersions(release.tag, current) <= 0) return say(`heavy-cmd ${current} is up to date (the newest release is ${release.tag})`);
  if (check) return say(`heavy-cmd ${release.tag} is out; this is ${current}: heavy-cmd update installs it`);
  await locked(where.dir, async () => {
    try {
      await install(where.dir, current, release, ports);
    } catch (error) {
      throw new Error(`update to ${release.tag} failed, ${current} kept: ${error instanceof Error ? error.message : String(error)}`);
    }
  });
  say(`heavy-cmd updated from ${current} to ${release.tag}`);
}

const DAY_MS = 86_400_000;

export type AutoPorts = { now: () => number; spawn: typeof spawn };

/**
 * After a command ends: at most once a day, `heavy-cmd update` detached in
 * the background, its output in `<install>/update.log`. Never in a dev
 * checkout, nor with `auto_update = off`; whatever fails is ignored, so the
 * command's output and exit stay its own. Whether it started one.
 */
export function autoUpdate(env = process.env, root = ROOT, ports: AutoPorts = { now: Date.now, spawn }): boolean {
  try {
    const where = installOf(root);
    if (where.kind !== "install" || !loadConfig(env).autoUpdate) return false;
    const stamp = path.join(where.dir, "update-check");
    const now = ports.now();
    let last = NaN;
    try {
      last = Date.parse(readFileSync(stamp, "utf8").trim());
    } catch {
      /* never checked */
    }
    if (Math.abs(now - last) < DAY_MS) return false;
    writeFileSync(stamp, `${new Date(now).toISOString()}\n`);
    const log = openSync(path.join(where.dir, "update.log"), "w");
    try {
      const bin = path.join(where.dir, "current", "bin", "heavy-cmd");
      const child = ports.spawn(process.execPath, [...NODE_FLAGS, bin, "update"], { detached: true, stdio: ["ignore", log, log], env });
      child.on("error", () => {});
      child.unref();
    } finally {
      closeSync(log);
    }
    return true;
  } catch {
    return false;
  }
}
