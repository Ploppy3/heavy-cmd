import { existsSync, readFileSync, readlinkSync } from "node:fs";
import path from "node:path";

/** This heavy-cmd's own folder, the one holding `bin/`, `dist/` and `package.json`: `src/` and `dist/` are both one level down. */
export const ROOT = path.resolve(import.meta.dirname, "..");

/** The release asset: `bin/`, `dist/` and `package.json` under one `heavy-cmd/` folder. */
export const ASSET = "heavy-cmd.tar.gz";

/** The flag every node that runs heavy-cmd gets, as `bin/heavy-cmd`'s shebang gives it: `node:sqlite` warns otherwise. */
export const NODE_FLAGS = ["--disable-warning=ExperimentalWarning"];

/**
 * What runs: a dev checkout, `bin/heavy-cmd` in a git clone; an install made
 * by install.sh, `<dir>/versions/<tag>` with `<dir>/current` linking the one
 * in use; or anything else, such as an unpacked release.
 */
export type Install = { kind: "dev"; root: string } | { kind: "install"; root: string; dir: string } | { kind: "other"; root: string };

export function installOf(root = ROOT): Install {
  if (existsSync(path.join(root, ".git"))) return { kind: "dev", root };
  const versions = path.dirname(root);
  if (path.basename(versions) === "versions" && existsSync(path.join(versions, "..", "current"))) return { kind: "install", root, dir: path.dirname(versions) };
  return { kind: "other", root };
}

/** The version `root`'s package.json carries, `0.1.0`. */
export const versionOf = (root = ROOT): string => (JSON.parse(readFileSync(path.join(root, "package.json"), "utf8")) as { version: string }).version;

/** `heavy-cmd v0.1.0`, ` (dev checkout)` after it in a git clone: what `--version` prints, and what an update checks a new one prints. */
export const versionLine = (root = ROOT) => `heavy-cmd v${versionOf(root)}${installOf(root).kind === "dev" ? " (dev checkout)" : ""}`;

/** The version folder `<dir>/current` links, by name: `v0.1.0`; null without the link. */
export function currentTag(dir: string): string | null {
  try {
    return path.basename(readlinkSync(path.join(dir, "current")));
  } catch {
    return null;
  }
}

/** `[1, 2, 3]` of `v1.2.3` or `1.2.3`; null for anything that is not a release version. */
export function parseVersion(text: string): [number, number, number] | null {
  const match = /^v?(\d+)\.(\d+)\.(\d+)$/.exec(text.trim());
  return match ? [Number(match[1]), Number(match[2]), Number(match[3])] : null;
}

/** Below 0 when `a` is older than `b`, 0 when they are one version, above 0 when `a` is newer. */
export function compareVersions(a: string, b: string): number {
  const [x, y] = [parseVersion(a), parseVersion(b)];
  if (!x || !y) throw new Error(`not a release version: ${x ? b : a}`);
  return x[0] - y[0] || x[1] - y[1] || x[2] - y[2];
}
