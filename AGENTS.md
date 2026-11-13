# AGENTS.md

This repo is `heavy-cmd`: a standalone CLI in Rust, one static binary for Linux and Windows (x86_64), that runs a heavy command — an install, a build, a test suite — once the machine has room, so the heavy commands of every agent and project on the machine share one queue. Its only command line is `heavy-cmd -- <command>...`. [README.md](README.md) documents its usage, exit codes and output.

Everything is built and tested on the developer's machine, in a Docker toolchain (`scripts/dev`): there is no CI, and a release is published from the machine (`scripts/release`). Run heavy commands — the checks, builds — through `heavy-cmd -- <command>`.

## Core principles

- **The bare minimum.** One command line, no flags, no settings, no subcommands, no record of past runs, no self-update, no migration or backward compatibility: a change replaces what was there. Add a feature only when a caller needs it.
- **A heavy command starts when the machine has room.** `heavy-cmd -- <command>` joins one machine-wide FIFO queue in `heavy.db` (`${XDG_STATE_HOME:-~/.local/state}/heavy-cmd`, `%LOCALAPPDATA%\heavy-cmd` on Windows); the oldest waiter starts once the last start was 10 seconds ago and, while others run, the whole machine's CPU usage over the last 10 seconds is under 60 %. With none running, the gap alone lets it in. Each decision is one `BEGIN IMMEDIATE` transaction, so two waiters never start in the same gap. A waiter or holder counts while its process lives: a dead pid, or a row from before the last boot, is pruned.
- **It runs in the foreground and exits as its command.** One argument runs through `sh -c` (`cmd.exe /d /s /c` on Windows), several are the argv (a bare name found with `PATHEXT` on Windows). Stdio is inherited; the exit is the command's (`128 + n` for a signal, `127` when it cannot start). One that cannot start within 900 s exits `75`. A signal while it waits stops the wait and ends heavy-cmd by that signal; once the command runs, `SIGTERM` and `SIGHUP` are passed on and `SIGINT` reaches it through its process group. On Windows the console's events stand for the signals (Ctrl+C exits 130), and the command runs in a job object that ends with heavy-cmd.
- **Inside another heavy-cmd it runs at once.** The command runs with `HEAVY_CMD_ID`, its queue id; a heavy-cmd started with it set skips the queue, since waiting on its own parent never ends.
- **Few dependencies.** `rusqlite` (SQLite compiled in), `libc` on Linux and `windows-sys` on Windows.

## Decisions

- **Rust, Linux and Windows:** one static binary per system, no runtime to install. Windows is built from Linux with mingw-w64 (`x86_64-pc-windows-gnu`, only system DLLs) and Linux with musl (`x86_64-unknown-linux-musl`, no dynamic linking). macOS is out of scope: `src/lib.rs` refuses to build there.
- **No CI, a Docker toolchain:** GitHub's runners are not used. `scripts/Dockerfile` holds Rust, mingw-w64, musl and Wine; `scripts/dev` runs any command in it with the repo at `/work` and the caches in `.cache/`, files owned by the user.
- **Windows is tested under Wine:** the suites run for Linux natively and for Windows under Wine. Wine runs no console events or job objects that matter, so those are untested on Windows, and `install.ps1` is only parsed. A real Windows machine is the remaining check of those.
- **Release assets are the bare binaries:** `heavy-cmd-linux-x86_64` and `heavy-cmd-windows-x86_64.exe`. The installers download them from GitHub's `releases/latest/download/`, so neither reads the API nor unpacks anything; running an installer again is the update.
- **No `--version`:** the usage, printed for any other command line, names the version on its first line.
- **The Windows shell is `cmd.exe /d /s /c "<line>"`**, the line passed verbatim, as npm runs scripts. Its unknown program exits 9009, which heavy-cmd passes on as the command's.
- **Windows' console events** map to signal numbers as its C runtime numbers them: Ctrl+C `SIGINT` (2), Ctrl+Break `SIGBREAK` (21), a closing `SIGHUP` (1); a stopped wait exits `128 +` that.
- **Exit codes** of heavy-cmd's own are sysexits': 64 usage, 75 no start; 1 for any other failure.
- **CPU counters are in ms**: Linux's from `/proc/stat` (user, nice, system and irq busy; idle), Windows' from `GetSystemTimes`.

## Files

| File | What it does |
|------|--------------|
| `src/main.rs` | Entry point: takes `-- <command>...` or prints the usage, catches the signals, runs the command and exits as it did. |
| `src/lib.rs` | The modules, and the crate the tests reach through it. |
| `src/fail.rs` | `Fail`: heavy-cmd's own failures and their exit codes. |
| `src/run.rs` | `run_heavy`: the command run once admitted, with `HEAVY_CMD_ID`; the waiting line. |
| `src/queue.rs` | `Heavy`: the queue in `heavy.db`, admission, liveness, status; where `heavy.db` lives. |
| `src/cpu.rs` | The machine's CPU usage over the last 10 seconds, from samples kept in `heavy.db`. |
| `src/format.rs` | Now in ms, durations, stderr lines. |
| `src/sys/` | What differs between systems: `unix.rs` (Linux) and `windows.rs` — CPU counters, boot time, pid liveness, the shell, signals or console events, the job object. |
| `install.sh`, `install.ps1` | Install, or update to, the newest release: `HEAVY_CMD_BIN_DIR` on Linux, `HEAVY_CMD_INSTALL_DIR` on Windows. |
| `scripts/Dockerfile`, `scripts/dev` | The toolchain, and the command that runs anything in it. |
| `scripts/check` | Every check: format, clippy and the suites, for Linux and Windows. |
| `scripts/pack` | Builds both release binaries into `dist/`, and checks each runs. |
| `scripts/release` | Publishes a release from the machine: check, pack, tag, `gh release create`. |
| `tests/` | The suites (`common/` their fakes); `line_budget.rs` keeps every `.rs` file to 450 lines. |

## Tests

`heavy-cmd -- scripts/check` — `cargo fmt --check`, clippy with warnings denied for both targets, then every suite for Linux and for Windows under Wine — is the floor for every change. The suites:

- `queue.rs`, `cpu.rs`: admission, the gap, the CPU target, liveness, the waiting line, on a fake clock, fake CPU counters and fake pids (`common::World`), each waiter a thread with its own connection.
- `cli.rs`: `heavy-cmd` as a process — the usage and 64, exits, nesting, a signal while waiting (Linux), `PATHEXT` (Windows), and the smoke test: two commands in a temp home, the second waiting out the 10 s gap.

Tests never touch the real queue: each runs on a temp state folder (`common::heavy_cmd` sets `XDG_STATE_HOME`, `LOCALAPPDATA` on Windows), with `HEAVY_CMD_ID` cleared.

## Workflow

0. Docker, and nothing else. `scripts/dev` builds the toolchain image the first time (and when `scripts/Dockerfile` changes).
1. Edit `src/`; `scripts/dev cargo build`, then `target/debug/heavy-cmd` runs on the machine as it is (point `XDG_STATE_HOME` at a temp folder when trying it by hand, so it stays off the machine's queue). `scripts/dev cargo test --test queue` runs one suite; add `--target x86_64-pc-windows-gnu` for Windows'.
2. `heavy-cmd -- scripts/check` before a change counts as done.

## Cutting a release

1. Bump `version` in `Cargo.toml` (then `scripts/dev cargo build` to update `Cargo.lock`), Commit, push `main`.
2. `heavy-cmd -- scripts/release`: it refuses a dirty or unpushed `main` or an existing tag, runs `scripts/check` and `scripts/pack`, pushes tag `v<version>` and creates the GitHub release with both binaries.
3. Check it: `gh release view v<x.y.z>` lists both assets; `install.sh` installs it.

Never move or reuse a published tag.
