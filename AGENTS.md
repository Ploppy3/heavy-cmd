# AGENTS.md

This repo is `heavy-cmd`: a standalone CLI in Rust, one static binary for Linux and Windows (x86_64), that runs a heavy command — an install, a build, a test suite — once the machine has room, so the heavy commands of every agent and project on the machine share one queue. It was extracted from the factory's `factory heavy` (Ploppy3/factory, v0.70.0, `7e77c16`), whose behaviour it keeps; nothing of it depends on the factory. [README.md](README.md) is the user's contract: usage, config, exit codes and the output callers match.

Everything is built and tested on the developer's machine, in a Docker toolchain (`scripts/dev`): there is no CI, and a release is published from the machine (`scripts/release`). Run heavy commands — the checks, builds — through `heavy-cmd -- <command>`.

## Core principles

- **A heavy command starts when the machine has room.** `heavy-cmd -- <command>` joins one machine-wide FIFO queue in `heavy.db` (`$HEAVY_CMD_HOME`, else `${XDG_STATE_HOME:-~/.local/state}/heavy-cmd`, `%LOCALAPPDATA%\heavy-cmd` on Windows); the oldest waiter starts once the last start was 10 seconds ago and, while others run, the whole machine's CPU usage over the last 10 seconds is under `cpu` (60 % unless set). With none running, the gap alone lets it in, whatever the CPU. Each decision is one `BEGIN IMMEDIATE` transaction, so two waiters never start in the same gap. A waiter or holder counts while its process lives: a dead pid, or a row from before the last boot, is pruned.
- **It runs in the foreground and exits as its command.** One argument runs through `sh -c` (`cmd.exe /d /s /c` on Windows), several are the argv (a bare name found with `PATHEXT` on Windows). Stdio is inherited; the exit is the command's (`128 + n` for a signal, `127` when it cannot start). One that cannot start within `wait` (900 s unless set) exits `75`. A signal while it waits stops the wait, is recorded, then ends heavy-cmd by that signal; once the command runs, `SIGTERM` and `SIGHUP` are passed on and `SIGINT` reaches it through its process group. On Windows the console's events stand for the signals (Ctrl+C exits 130), and the command runs in a job object that ends with heavy-cmd.
- **Inside another heavy-cmd it runs at once.** The command runs with `HEAVY_CMD_ID`, its queue id; a heavy-cmd started with it set skips the queue, since waiting on its own parent never ends, and is recorded `inside` that id.
- **Every run is recorded once it is over.** In `heavy.db`'s `runs`: command, cwd, host, pid, project, label, agent, queued/started/ended times, exit, outcome (`ran`, `gave up`, `stopped while waiting`), its `heavy_id` and the `inside` it ran in. A write that fails never changes the command's outcome, only one stderr line; each write forgets runs over 30 days old. `heavy-cmd logs` reads it read-only.
- **Its output is a contract.** The waiting line contains `every heavy slot is busy`, said once on stderr; exit `75` means it never ran; `HEAVY_CMD_AGENT` names the agent; `HEAVY_CMD_ID` nests. Callers (the factory among them) match these: change them only with a major version.
- **Settings: defaults, then file, then env, then flags.** `${XDG_CONFIG_HOME:-~/.config}/heavy-cmd/config` (`%APPDATA%\heavy-cmd\config` on Windows: `cpu`, `wait`, `auto_update`), then `HEAVY_CMD_CPU`, `HEAVY_CMD_WAIT`, `HEAVY_CMD_AUTO_UPDATE`, then `--wait`. A value out of its bounds (cpu 10–100, wait 0–86400, auto_update on/off), an unknown key or a malformed line is refused naming where and why (exit 78), never guessed.
- **The install is not a working copy.** `install.sh` (Linux) and `install.ps1` (Windows) put the release's binary in `<install>/bin/heavy-cmd[.exe]` beside a `<install>/.heavy-cmd-install` marker; Linux links `~/.local/bin/heavy-cmd` to it, Windows puts `<install>\bin` on the user's PATH. `heavy-cmd update` only ever moves to a newer release, puts the new binary in place once its `--version` names it, and keeps the one before as `heavy-cmd.old[.exe]`; a dev build (under a git clone with `Cargo.toml`) is refused and never auto-updates. The auto update runs `heavy-cmd update` detached, at most once a day (`<install>/update-check`), after a command ends, so it never delays a command nor touches its output or exit.
- **Few dependencies, no factory.** `rusqlite` (SQLite compiled in), `serde_json`, `chrono` (local time, ISO times), `libc` on Linux and `windows-sys` on Windows. Downloads go through the system's `curl` and `tar`, so no TLS or archive crate. No `FACTORY_*` variable, no `~/.factory` path.

## Decisions

What the brief left open, and what was chosen:

- **Rust, Linux and Windows:** one static binary per system, no runtime to install. Windows is built from Linux with mingw-w64 (`x86_64-pc-windows-gnu`, only system DLLs) and Linux with musl (`x86_64-unknown-linux-musl`, no dynamic linking). macOS is out of scope: `src/lib.rs` refuses to build there.
- **No CI, a Docker toolchain:** GitHub's runners are not used. `scripts/Dockerfile` holds Rust, mingw-w64, musl and Wine; `scripts/dev` runs any command in it with the repo at `/work` and the caches in `.cache/`, files owned by the user. Nothing else is installed on the machine.
- **Windows is tested under Wine:** the suites run for Linux natively and for Windows under Wine. Wine runs no console events, job objects that matter, git, `curl.exe` or `tar.exe`: what needs them is `#[cfg(unix)]` or untested on Windows, and `install.ps1` is only parsed. A real Windows machine is the remaining check of those.
- **Release artifacts:** `heavy-cmd-linux-x86_64.tar.gz` and `heavy-cmd-windows-x86_64.zip`, each a `heavy-cmd/` folder with `bin/`, `README.md` and `LICENSE`. A binary knows its own asset's name (`install::asset`).
- **Install layout:** one binary and the one before it (`.old`), switched with renames: atomic on Linux; on Windows, the running one is renamed away first, and a `.old` still running is renamed aside (`.discarded-*`) and removed by a later update.
- **The Windows shell is `cmd.exe /d /s /c "<line>"`**, the line passed verbatim, as npm runs scripts. Its unknown program exits 9009, which heavy-cmd passes on as the command's.
- **Windows' console events** map to signal numbers as its C runtime numbers them: Ctrl+C `SIGINT` (2), Ctrl+Break `SIGBREAK` (21), a closing `SIGHUP` (1); a stopped wait exits `128 +` that.
- **Releases are read** from GitHub's `releases/latest` API (`HEAVY_CMD_RELEASES_API` overrides it, for a mirror or the tests), by both installers and `update`; drafts and prereleases are never installed.
- **Exit codes** of heavy-cmd's own errors are sysexits': 64 usage, 78 config, 75 no start (kept from the factory); 1 for any other failure, such as an update.
- **`wait` is 0–86400:** 0 (try once, give up at once) is valid everywhere.
- **`--agent` is handed on** to the command as `HEAVY_CMD_AGENT`, so nested heavy-cmds record the same agent.
- **`status` keeps a CPU sample** as it reads, as `factory heavy status` did, so a status read alone measures the CPU after 10 s; `logs` opens the record read-only.
- **CPU counters are in ms**: Linux's from `/proc/stat` (user, nice, system and irq busy; idle), Windows' from `GetSystemTimes`.
- **The auto-update stamp and log live in the install** (`update-check`, `update.log`), not the state folder: the install is what updates, whatever `HEAVY_CMD_HOME` a command uses.
- **`--check` exits 0** either way and says which in one line.
- **The project label** asks git (`remote get-url origin`, `rev-parse --show-toplevel`); without git it is the cwd's name.

## Files

| File | What it does |
|------|--------------|
| `src/main.rs` | Entry point: parses the command line, dispatches, maps a failure to its exit code, starts the auto update after a run. |
| `src/lib.rs` | The modules, and the crates the tests reach through it. |
| `src/cli.rs` | Flag parsing (`--key value`, `--key=value`, switches, `--` and the command after it), `--json` output. |
| `src/fail.rs` | `Fail`: heavy-cmd's own failures and their exit codes. |
| `src/commands.rs` | `heavy-cmd -- <command>`, `status` and `logs`: their options, lines and JSON. |
| `src/run.rs` | `run_heavy`: the command run once admitted, its env (`HEAVY_CMD_ID`, `HEAVY_CMD_AGENT`) and its record; `heavily`, the same for any work. |
| `src/queue.rs` | `Heavy`: the queue in `heavy.db`, admission, liveness, status, the record's writes; `read_runs`, its read-only reads. |
| `src/cpu.rs` | The machine's CPU usage over the last 10 seconds, from samples kept in `heavy.db`. |
| `src/runs.rs` | The `runs` table: recording, forgetting after 30 days, filtering, `--since`. |
| `src/config.rs` | `Env`, the config file, env and flag settings and their bounds; where state and config live. |
| `src/project.rs` | The project a run is recorded under, from git. |
| `src/install.rs` | What runs (dev build, install, other), its version and `--version` line, its release asset, version comparison. |
| `src/update.rs` | `heavy-cmd update [--check]` against GitHub's releases, and the daily auto update. |
| `src/format.rs` | ISO and local times, durations, stdout and stderr lines. |
| `src/sys/` | What differs between systems: `unix.rs` (Linux) and `windows.rs` — CPU counters, boot time, pid liveness, the shell, signals or console events, detaching, the job object, the binary's switch. |
| `install.sh`, `install.ps1` | Install the newest release, idempotently: `HEAVY_CMD_INSTALL_DIR`, and `HEAVY_CMD_BIN_DIR` on Linux. |
| `scripts/Dockerfile`, `scripts/dev` | The toolchain, and the command that runs anything in it. |
| `scripts/check` | Every check: format, clippy and the suites, for Linux and Windows. |
| `scripts/pack` | Builds and packs both release assets into `dist/`, and checks each unpacked binary runs. |
| `scripts/release` | Publishes a release from the machine: check, pack, tag, `gh release create`. |
| `tests/` | The suites (`common/` their fakes, `update/` the Linux-only update cases); `line_budget.rs` keeps every `.rs` file to 450 lines. |

## Tests

`heavy-cmd -- scripts/check` — `cargo fmt --check`, clippy with warnings denied for both targets, then every suite for Linux and for Windows under Wine — is the floor for every change. The suites:

- `queue.rs`, `record.rs`, `cpu.rs`: the factory's heavy suites, ported case by case — admission, the gap, the CPU target, liveness, the record, `logs` — on a fake clock, fake CPU counters and fake pids (`common::World`), each waiter a thread with its own connection. The factory's "own work" case became "work that is not a command".
- `config.rs`, `project.rs`, `commands.rs`: settings and their bounds (and Windows' folders), the project label from git, the run's options and `status --json`'s fields.
- `update.rs`: the binary's switch, the auto update, what counts as an install; `update/releases.rs` (Linux) runs `update` on a fake install against tarballs and a local release server, and the detached start.
- `install.rs` (Linux): `install.sh` end to end against a local release server.
- `cli.rs`: `heavy-cmd` as a process — exits, the waiting line and 75, nesting, a signal while waiting (Linux), `PATHEXT` (Windows), and the smoke test: two commands in a temp home, the second waiting out the 10 s gap, and `logs` showing both.

Tests never touch the real queue, config or install: each runs on a temp state and config folder (`common::heavy_cmd`, explicit `Env`s), with `HEAVY_CMD_ID` and `HEAVY_CMD_AGENT` cleared.

## Workflow

0. Docker, and nothing else. `scripts/dev` builds the toolchain image the first time (and when `scripts/Dockerfile` changes).
1. Edit `src/`; `scripts/dev cargo build`, then `target/debug/heavy-cmd` runs on the machine as it is (point it at a temp `HEAVY_CMD_HOME` when trying it by hand, so it stays off the machine's queue). `scripts/dev cargo test --test queue` runs one suite; add `--target x86_64-pc-windows-gnu` for Windows'.
2. `heavy-cmd -- scripts/check` before a change counts as done.

## Cutting a release

1. Bump `version` in `Cargo.toml` (then `scripts/dev cargo build` to update `Cargo.lock`), following semver; a change to the output contract is a major one. Commit, push `main`.
2. `heavy-cmd -- scripts/release`: it refuses a dirty or unpushed `main` or an existing tag, runs `scripts/check` and `scripts/pack`, pushes tag `v<version>` and creates the GitHub release with both assets.
3. Check it: `gh release view v<x.y.z>` lists both assets; `heavy-cmd update --check` on an install sees it.

Never move or reuse a published tag: installs only go forward, and one already on it would never see the change.
