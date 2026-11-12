import { execFileSync } from "node:child_process";
import { mkdirSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { createServer, type Server } from "node:http";
import type { AddressInfo } from "node:net";
import path from "node:path";
import { ROOT } from "../src/install.ts";
import { pack } from "../scripts/pack.ts";

/** A release tarball in `dir` of this build as `tag`; with `bin`, one whose `bin/heavy-cmd` is that script instead. */
export function release(dir: string, tag: string, bin?: string): string {
  const out = path.join(dir, `${tag}${bin ? "-broken" : ""}.tar.gz`);
  if (!bin) {
    pack(ROOT, out, tag.slice(1));
    return out;
  }
  const stage = path.join(dir, "stage", "heavy-cmd");
  mkdirSync(path.join(stage, "bin"), { recursive: true });
  writeFileSync(path.join(stage, "bin", "heavy-cmd"), bin);
  writeFileSync(path.join(stage, "package.json"), JSON.stringify({ version: tag.slice(1) }));
  execFileSync("tar", ["-czf", out, "-C", path.dirname(stage), "heavy-cmd"]);
  rmSync(path.dirname(stage), { recursive: true });
  return out;
}

/**
 * GitHub's releases as a local server: `/latest` answers as the API's
 * `releases/latest` for the release `newest` holds, `/no-asset` with a release
 * with no tarball, and the tarball is served from `/download/`.
 */
export async function serveReleases(): Promise<{ api: string; base: string; newest: { tag: string; tarball: string } | null; close: () => void }> {
  const state = { api: "", base: "", newest: null as { tag: string; tarball: string } | null, close: () => server.close() };
  const server: Server = createServer((request, response) => {
    const tag = state.newest?.tag;
    if (request.url === "/latest" && state.newest) {
      response.setHeader("content-type", "application/json");
      response.end(JSON.stringify({ tag_name: tag, assets: [{ name: "heavy-cmd.tar.gz", browser_download_url: `${state.base}/download/${tag}/heavy-cmd.tar.gz` }] }, null, 2));
    } else if (request.url === "/no-asset") response.end(JSON.stringify({ tag_name: "v0.2.0", assets: [] }));
    else if (request.url === `/download/${tag}/heavy-cmd.tar.gz` && state.newest) response.end(readFileSync(state.newest.tarball));
    else response.writeHead(404).end();
  });
  await new Promise<void>((resolve) => server.listen(0, "127.0.0.1", resolve));
  state.base = `http://127.0.0.1:${(server.address() as AddressInfo).port}`;
  state.api = `${state.base}/latest`;
  return state;
}
