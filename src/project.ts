import { execFileSync } from "node:child_process";
import path from "node:path";

/** git's answer in `cwd`, trimmed; null when git fails, is missing or says nothing. */
function git(cwd: string, args: string[]): string | null {
  try {
    return execFileSync("git", ["-C", cwd, ...args], { encoding: "utf8", stdio: ["ignore", "pipe", "ignore"] }).trim() || null;
  } catch {
    return null;
  }
}

/** A remote's repository name: `git@github.com:o/heavy-cmd.git`, `https://host/group/heavy-cmd/` and `/srv/git/heavy-cmd.git` are `heavy-cmd`. */
export function repoName(url: string): string | null {
  const last = url.trim().replace(/[/\\]+$/, "").split(/[/:\\]/).pop() ?? "";
  return last.replace(/\.git$/, "") || null;
}

/**
 * The project a command in `cwd` is recorded under: the repository name of
 * the git `origin` remote, else the git top-level folder's name, else the
 * cwd's. Origin comes first: a worktree's folder is not its project's name.
 */
export function projectOf(cwd: string): string {
  const origin = git(cwd, ["remote", "get-url", "origin"]);
  const name = origin && repoName(origin);
  if (name) return name;
  const top = git(cwd, ["rev-parse", "--show-toplevel"]);
  return path.basename(top ?? cwd) || cwd;
}
