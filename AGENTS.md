# AGENTS.md

This repo is `heavy-cmd`: a standalone CLI, TypeScript on Node 26 and its built-in SQLite, compiled to JavaScript by `tsc`, that runs a heavy command — an install, a build, a test suite — once the machine has room, so the heavy commands of every agent and project on the machine share one queue. It was extracted from the factory's `factory heavy` (Ploppy3/factory, v0.70.0, `7e77c16`), whose behaviour it keeps; nothing of it depends on the factory. [README.md](README.md) is the user's contract: usage, config, exit codes and the output callers match.

Command checks: `npm run check` (build, typecheck, unit tests, line budget), `npm run check:lines` on its own. Run heavy commands — the suite, builds — through `heavy-cmd -- <command>`.

## Core principles

- **A heavy command starts when the machine has room.** `heavy-cmd -- <command>` joins one machine-wide FIFO queue in `heavy.db` (`$HEAVY_CMD_HOME`, else `${XDG_STATE_HOME:-~/.local/state}/heavy-cmd`); the oldest waiter starts once the last start was 10 seconds ago and, while others run, the whole machine's CPU usage over the last 10 seconds is under `cpu` (60 % unless set). With none running, the gap alone lets it in, whatever the CPU. Each decision is one `BEGIN IMMEDIATE` transaction, so two waiters never start in the same gap. A waiter or holder counts while its process lives: a dead pid, or a row from before the last boot, is pruned.
- **It runs in the foreground and exits as its command.** One argument runs through `sh -c`, several are the argv. Stdio is inherited; the exit is the command's (`128 + n` for a signal, `127` when it cannot start). One that cannot start within `wait` (900 s unless set) exits `75`. A signal while it waits stops the wait, is recorded, then ends heavy-cmd by that signal; once the command runs, `SIGTERM` and `SIGHUP` are passed on and `SIGINT` reaches it through its process group.
- **Inside another heavy-cmd it runs at once.** The command runs with `HEAVY_CMD_ID`, its queue id; a heavy-cmd started with it set skips the queue, since waiting on its own parent never ends, and is recorded `inside` that id.
- **Every run is recorded once it is over.** In `heavy.db`'s `runs`: command, cwd, host, pid, project, label, agent, queued/started/ended times, exit, outcome (`ran`, `gave up`, `stopped while waiting`), its `heavy_id` and the `inside` it ran in. A write that fails never changes the command's outcome, only one stderr line; each write forgets runs over 30 days old. `heavy-cmd logs` reads it read-only.
- **Its output is a contract.** The waiting line contains `every heavy slot is busy`, said once on stderr; exit `75` means it never ran; `HEAVY_CMD_AGENT` names the agent; `HEAVY_CMD_ID` nests. Callers (the factory among them) match these: change them only with a major version.
- **Settings: defaults, then file, then env, then flags.** `${XDG_CONFIG_HOME:-~/.config}/heavy-cmd/config` (`cpu`, `wait`, `auto_update`), then `HEAVY_CMD_CPU`, `HEAVY_CMD_WAIT`, `HEAVY_CMD_AUTO_UPDATE`, then `--wait`. A value out of its bounds (cpu 10–100, wait 0–86400, auto_update on/off), an unknown key or a malformed line is refused naming where and why (exit 78), never guessed.
- **The install is not a working copy.** `install.sh` unpacks the release tarball into `<install>/versions/<tag>` and points `<install>/current` at it; `~/.local/bin/heavy-cmd` links `current/bin/heavy-cmd`. `heavy-cmd update` only ever moves to a newer release, switches `current` with one rename once the new one's `--version` names it, and keeps the version before; a dev checkout (`.git` beside `bin/`) is refused and never auto-updates. The auto update runs `heavy-cmd update` detached, at most once a day (`<install>/update-check`), after a command ends, so it never delays a command nor touches its output or exit.
- **No runtime dependencies, no factory.** Node's own modules only; no `FACTORY_*` variable, no `~/.factory` path.

## Decisions

What the brief left open, and what was chosen:

- **Release artifact:** a tarball of the `tsc` build, `bin/`, `package.json`, `README.md` and `LICENSE` under `heavy-cmd/`, named `heavy-cmd.tar.gz` in every release. No bundler, so no build dependency beyond `tsc`, and the tree that runs is the one the suite tests. `scripts/pack.ts` makes it and checks the unpacked copy runs.
- **Install layout:** `versions/<tag>` plus a `current` symlink, so an update is one atomic rename and a failure leaves the old version in place; `update` keeps the new one and the one before, and drops older ones.
- **Releases are read** from GitHub's `releases/latest` API (`HEAVY_CMD_RELEASES_API` overrides it, for a mirror or the tests), by both `install.sh` and `update`; drafts and prereleases are never installed.
- **Exit codes** of heavy-cmd's own errors are sysexits': 64 usage, 78 config, 75 no start (kept from the factory); 1 for any other failure, such as an update.
- **`wait` is 0–86400:** the factory's config allowed 1–86400 and its `--wait` flag 0; 0 (try once, give up at once) is useful and now valid everywhere.
- **`--agent` is handed on** to the command as `HEAVY_CMD_AGENT`, so nested heavy-cmds record the same agent.
- **`status` keeps a CPU sample** as it reads, as `factory heavy status` did, so a status read alone measures the CPU after 10 s; `logs` opens the record read-only.
- **The auto-update stamp and log live in the install** (`update-check`, `update.log`), not the state folder: the install is what updates, whatever `HEAVY_CMD_HOME` a command uses.
- **`--check` exits 0** either way and says which in one line.
- **The project label** asks git (`remote get-url origin`, `rev-parse --show-toplevel`); without git it is the cwd's name.

## Files

| File | What it does |
|------|--------------|
| `src/main.ts` | Entry point: parses the command line, dispatches, maps an error to its exit code, starts the auto update after a run. |
| `src/cli.ts` | Flag parsing (`--key value`, `--key=value`, switches, `--` and the command after it), exit codes, `UsageError`, `--json` output. |
| `src/commands.ts` | `heavy-cmd -- <command>`, `status` and `logs`: their options, lines and JSON. |
| `src/run.ts` | `runHeavy`: the command run once admitted, its signals, its env (`HEAVY_CMD_ID`, `HEAVY_CMD_AGENT`) and its record; `heavily`, the same for any work. |
| `src/queue.ts` | `Heavy`: the queue in `heavy.db`, admission, liveness, status, the record's writes; `readRuns`, its read-only reads. |
| `src/cpu.ts` | The machine's CPU usage over the last 10 seconds, from samples kept in `heavy.db`. |
| `src/runs.ts` | The `runs` table: recording, forgetting after 30 days, filtering, `--since`. |
| `src/config.ts` | The config file, env and flag settings and their bounds; where state and config live. |
| `src/project.ts` | The project a run is recorded under, from git. |
| `src/install.ts` | What runs (dev checkout, install, other), its version and `--version` line, version comparison. |
| `src/update.ts` | `heavy-cmd update [--check]` against GitHub's releases, and the daily auto update. |
| `src/format.ts` | Durations, clock times, pid liveness. |
| `bin/heavy-cmd` | Runs `dist/main.js`, or `src/main.ts` while there is no build (Node strips its types, which is why `src/` keeps to erasable syntax). |
| `dist/` | What `npm run build` (`tsconfig.build.json`) emits from `src/`; git-ignored. |
| `install.sh` | Installs the newest release, idempotently: `HEAVY_CMD_INSTALL_DIR`, `HEAVY_CMD_BIN_DIR`. |
| `scripts/check-file-length.ts` | The line budget `npm run check` enforces: 450 lines per TypeScript file in `src/` and `scripts/`. |
| `scripts/pack.ts` | Packs and checks `heavy-cmd.tar.gz`, the release asset (`npm run pack:release`). |
| `tests/` | Vitest suites (`vitest.config.ts` builds `dist/` first, `tests/setup.ts` points every suite at a temp state, config and no auto update); `releases.ts` packs releases and serves them as GitHub's API would. |
| `.github/workflows/` | `ci.yml` runs `npm run check` on `main` and pull requests; `release.yml` publishes a release for a `v*` tag. |

## Tests

`npm run check` — build, typecheck, the Vitest suites, then `check:lines` — is the floor for every change. The suites:

- `cpu.test.ts`, `queue.test.ts`: the factory's heavy suites, ported case by case — admission, the gap, the CPU target, liveness, the record, `logs` — on a fake clock and fake CPU counters. The factory's "own work" case became "work that is not a command"; a case for the agent was added.
- `config.test.ts`, `project.test.ts`, `commands.test.ts`: settings and their bounds, the project label from git, the run's options and `status --json`'s fields.
- `update.test.ts`, `install.test.ts`: `update` and the auto update on a fake install, and `install.sh` end to end against a local release server.
- `cli.test.ts`: `bin/heavy-cmd` as a process — exits, the waiting line and 75, nesting, a signal while waiting, and the smoke test: two commands in a temp home, the second waiting out the 10 s gap, and `logs` showing both.

Tests never touch the real queue, config or install: every suite runs with a temp `HEAVY_CMD_HOME` and `XDG_*` folders, and `HEAVY_CMD_ID` and `HEAVY_CMD_AGENT` cleared, since the suite itself runs inside a heavy-cmd.

## Workflow

0. First time, with Node 26 or later: `npm ci`.
1. Edit `src/`; `./bin/heavy-cmd` runs the build after `npm run build` (or the source when there is no `dist/`). Point it at a temp `HEAVY_CMD_HOME` when trying it by hand, so it stays off the machine's queue.
2. `heavy-cmd -- npm run check` before a change counts as done.

## Cutting a release

1. Bump `version` in `package.json` (and `package-lock.json`: `npm version <x.y.z> --no-git-tag-version`), following semver; a change to the output contract is a major one.
2. `heavy-cmd -- npm run check`, commit, push `main`.
3. `git tag v<x.y.z> && git push origin v<x.y.z>`. The `release` workflow checks the tag is `package.json`'s version, runs `npm run check`, packs `heavy-cmd.tar.gz` and publishes the release with it.
4. Check it: `gh release view v<x.y.z>` lists `heavy-cmd.tar.gz`; `heavy-cmd update --check` on an install sees it.

Never move or reuse a published tag: installs only go forward, and one already on it would never see the change.
