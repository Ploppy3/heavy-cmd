#!/usr/bin/env node
// Packs the release asset, heavy-cmd.tar.gz: bin/, dist/, package.json, README.md and LICENSE under one
// heavy-cmd/ folder, from a build (`npm run build` first). Run as a script, it then unpacks the tarball
// elsewhere and checks that it runs and names package.json's version.
//
//   npm run pack:release        writes ./heavy-cmd.tar.gz
import { execFileSync } from "node:child_process";
import { cpSync, existsSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import path from "node:path";

const ROOT = path.resolve(import.meta.dirname, "..");

/** Packs `root`'s build into `out`, its package.json's version `version` when given. */
export function pack(root: string, out: string, version?: string): void {
  if (!existsSync(path.join(root, "dist", "main.js"))) throw new Error(`${root} has no build: run npm run build first`);
  const stage = mkdtempSync(path.join(tmpdir(), "heavy-cmd-pack-"));
  try {
    const dest = path.join(stage, "heavy-cmd");
    for (const part of ["bin", "dist", "README.md", "LICENSE"]) {
      if (existsSync(path.join(root, part))) cpSync(path.join(root, part), path.join(dest, part), { recursive: true });
    }
    const manifest = JSON.parse(readFileSync(path.join(root, "package.json"), "utf8")) as Record<string, unknown>;
    writeFileSync(path.join(dest, "package.json"), `${JSON.stringify({ ...manifest, version: version ?? manifest.version }, null, 2)}\n`);
    execFileSync("tar", ["-czf", path.resolve(out), "-C", stage, "heavy-cmd"]);
  } finally {
    rmSync(stage, { recursive: true, force: true });
  }
}

if (import.meta.main) {
  const out = path.join(ROOT, "heavy-cmd.tar.gz");
  pack(ROOT, out);
  const version = (JSON.parse(readFileSync(path.join(ROOT, "package.json"), "utf8")) as { version: string }).version;
  const check = mkdtempSync(path.join(tmpdir(), "heavy-cmd-unpack-"));
  try {
    execFileSync("tar", ["-xzf", out, "-C", check, "--strip-components=1"]);
    const said = execFileSync(process.execPath, ["--disable-warning=ExperimentalWarning", path.join(check, "bin", "heavy-cmd"), "--version"], { encoding: "utf8" }).trim();
    if (said !== `heavy-cmd v${version}`) throw new Error(`the packed heavy-cmd says "${said}", not heavy-cmd v${version}`);
    console.log(`✓ ${path.relative(process.cwd(), out)}: ${said}`);
  } finally {
    rmSync(check, { recursive: true, force: true });
  }
}
