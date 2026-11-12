import { afterEach, beforeEach, describe, expect, test } from "vitest";
import { execFileSync } from "node:child_process";
import { mkdirSync, mkdtempSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import path from "node:path";
import { projectOf, repoName } from "../src/project.ts";

let dir: string;

const git = (cwd: string, ...args: string[]) => execFileSync("git", ["-C", cwd, ...args], { stdio: "ignore" });

beforeEach(() => {
  dir = mkdtempSync(path.join(tmpdir(), "heavy-cmd-project-"));
});

afterEach(() => rmSync(dir, { recursive: true, force: true }));

describe("the project a command is recorded under", () => {
  test("a remote's repository name, whatever its form", () => {
    expect(repoName("git@github.com:Ploppy3/heavy-cmd.git")).toBe("heavy-cmd");
    expect(repoName("https://gitlab.example.com/group/sub/heavy-cmd/")).toBe("heavy-cmd");
    expect(repoName("ssh://git@host:2222/heavy-cmd")).toBe("heavy-cmd");
    expect(repoName("/srv/git/heavy-cmd.git")).toBe("heavy-cmd");
    expect(repoName("")).toBeNull();
  });

  test("origin's repository name first: a worktree's folder is not the project", () => {
    const repo = path.join(dir, "checkout");
    mkdirSync(repo);
    git(repo, "init", "-q");
    git(repo, "remote", "add", "origin", "git@github.com:acme/api.git");
    git(repo, "-c", "user.name=t", "-c", "user.email=t@t", "commit", "-q", "--allow-empty", "-m", "init");
    const worktree = path.join(dir, "task-73-fix-login");
    git(repo, "worktree", "add", "-q", worktree);
    mkdirSync(path.join(worktree, "src"));
    expect(projectOf(path.join(worktree, "src"))).toBe("api");
    expect(projectOf(repo)).toBe("api");
  });

  test("with no origin, the git top-level folder's name; outside git, the cwd's name", () => {
    const repo = path.join(dir, "scratch");
    mkdirSync(path.join(repo, "deep", "er"), { recursive: true });
    git(repo, "init", "-q");
    expect(projectOf(path.join(repo, "deep", "er"))).toBe("scratch");
    const plain = path.join(dir, "plain");
    mkdirSync(plain);
    expect(projectOf(plain)).toBe("plain");
  });
});
