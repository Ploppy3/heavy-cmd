# AGENTS.md

This repo is `heavy-cmd`: a standalone CLI in Rust, one static binary for Linux and Windows (x86_64), that runs a heavy command — an install, a build, a test suite — once the machine has room, so the heavy commands of every agent and project on the machine share one queue. Its command lines are `heavy-cmd -- <command>...`, `heavy-cmd logs` and `heavy-cmd update`. [README.md](README.md) documents its usage, exit codes and output.

Everything is built and tested on the developer's machine, in a Docker toolchain (`scripts/dev`): there is no CI, and a release is published from the machine (`scripts/release`). Run heavy commands — the checks, builds — through `heavy-cmd -- <command>`.

## Core principles

- **The bare minimum.** Three command lines, no flags, no settings, no migration or backward compatibility: a change replaces what was there. Add a feature only when a caller needs it.
- **A heavy command starts when the machine has room.** `heavy-cmd -- <command>` joins one machine-wide FIFO queue in `heavy.db` (`${XDG_STATE_HOME:-~/.local/state}/heavy-cmd`, `%LOCALAPPDATA%\heavy-cmd` on Windows); the oldest waiter starts once the last start was 10 seconds ago and, while others run, the whole machine's CPU usage over the last 10 seconds is under 60 %. With none running, the gap alone lets it in. Each decision is one `BEGIN IMMEDIATE` transaction, so two waiters never start in the same gap. A waiter or holder counts while its process lives: a dead pid, or a row from before the last boot, is pruned.
- **It runs in the foreground and exits as its command.** One argument runs through `sh -c` (`cmd.exe /d /s /c` on Windows), several are the argv (a bare name found with `PATHEXT` on Windows). Stdio is inherited; the exit is the command's (`128 + n` for a signal, `127` when it cannot start). One that cannot start within 900 s exits `75`. A signal while it waits stops the wait and ends heavy-cmd by that signal; once the command runs, `SIGTERM` and `SIGHUP` are passed on and `SIGINT` reaches it through its process group. On Windows the console's events stand for the signals (Ctrl+C exits 130), and the command runs in a job object that ends with heavy-cmd.
- **Every run is recorded once it is over.** In `heavy.db`'s `runs`: command, cwd, queued/started/ended times, exit and outcome (`ran`, `gave up`, `stopped while waiting`). A write that fails never changes the command's outcome, only one stderr line; each write forgets runs over 30 days old. `heavy-cmd logs` lists the last 50, reading `heavy.db` read-only.
- **A release's binary updates itself, only to a verified newer release.** After a command ends (not one a signal stopped), at most once an hour on the machine (`update_check` in `heavy.db`'s `meta`, taken in one `BEGIN IMMEDIATE` transaction), it starts `heavy-cmd update` detached, its output in `update.log` beside `heavy.db`; nothing of it touches the command's output or exit. `update` reads the newest tag from the `releases/latest` redirect and, only when it is newer, fetches the release's `SHA256SUMS` and this system's binary; the binary is written beside the one running only once its SHA-256 is the one `SHA256SUMS` names, and takes its place only once, run, its usage names that tag. Anything else leaves the binary as it was.
- **Inside another heavy-cmd it runs at once.** The command runs with `HEAVY_CMD_ID`, its queue id; a heavy-cmd started with it set skips the queue, since waiting on its own parent never ends.
- **Few dependencies.** `rusqlite` (SQLite compiled in), `chrono` (local time in `logs`), `libc` on Linux and `windows-sys` on Windows. Downloads go through the system's `curl` (`System32\curl.exe` on Windows), so no TLS crate; SHA-256 is `src/sha256.rs`, so no crypto crate.

## Decisions

- **Rust, Linux and Windows:** one static binary per system, no runtime to install. Windows is built from Linux with mingw-w64 (`x86_64-pc-windows-gnu`, only system DLLs) and Linux with musl (`x86_64-unknown-linux-musl`, no dynamic linking). macOS is out of scope: `src/lib.rs` refuses to build there.
- **No CI, a Docker toolchain:** GitHub's runners are not used. `scripts/Dockerfile` holds Rust, mingw-w64, musl and Wine; `scripts/dev` runs any command in it with the repo at `/work` and the caches in `.cache/`, files owned by the user.
- **Windows is tested under Wine:** the suites run for Linux natively and for Windows under Wine. Wine runs no console events or job objects that matter, so those are untested on Windows, and `install.ps1` is only parsed. A real Windows machine is the remaining check of those.
- **Release assets are the bare binaries and their sums:** `heavy-cmd-linux-x86_64`, `heavy-cmd-windows-x86_64.exe` and `SHA256SUMS` (`sha256sum`'s format). The installers download them from GitHub's `releases/latest/download/` and refuse a binary whose SHA-256 is not the one named; the update reads `releases/download/<tag>/`. Nothing reads GitHub's API (60 requests an hour per IP without a token) or unpacks anything.
- **Only a release's binary updates itself:** the `self-update` Cargo feature, which only `scripts/pack` turns on. A dev or test build never checks, and its `heavy-cmd update` refuses with 1, so the suites test the update against fake releases (`update::UpdatePorts`).
- **SHA256SUMS guards against a broken or mismatched download, not a forged release:** it comes from the same release as the binary, so whoever can publish a release can publish both. HTTPS (`--proto =https`) guards the way there.
- **The switch:** on Linux one rename over the binary, so a heavy-cmd running keeps the one it started with. On Windows a running `.exe` cannot be replaced but can be renamed: it becomes `heavy-cmd.<pid>.old`, removed by a later update or `install.ps1`, as `install.ps1` does.
- **The background update on Windows** starts with `CREATE_NO_WINDOW | CREATE_NEW_PROCESS_GROUP` (and out of heavy-cmd's job when it allows): a console of its own that shows no window, so curl shows none either, and the console closing does not end it. On Linux it starts in a session of its own (`setsid`).
- **No `--version`:** the usage, printed for any other command line, names the version on its first line.
- **`logs` takes no flags:** the last 50 runs, each with its cwd (the home as `~`) standing for its project.
- **The Windows shell is `cmd.exe /d /s /c "<line>"`**, the line passed verbatim, as npm runs scripts. Its unknown program exits 9009, which heavy-cmd passes on as the command's.
- **Windows' console events** map to signal numbers as its C runtime numbers them: Ctrl+C `SIGINT` (2), Ctrl+Break `SIGBREAK` (21), a closing `SIGHUP` (1); a stopped wait exits `128 +` that.
- **Exit codes** of heavy-cmd's own are sysexits': 64 usage, 75 no start; 1 for any other failure, an update's among them.
- **CPU counters are in ms**: Linux's from `/proc/stat` (user, nice, system and irq busy; idle), Windows' from `GetSystemTimes`.

## Files

| File | What it does |
|------|--------------|
| `src/main.rs` | Entry point: takes `-- <command>...`, `logs` or `update`, else prints the usage; catches the signals, runs the command, starts the hourly update check and exits as the command did. |
| `src/lib.rs` | The modules, and the crate the tests reach through it. |
| `src/fail.rs` | `Fail`: heavy-cmd's own failures and their exit codes. |
| `src/run.rs` | `run_heavy`: the command run once admitted, with `HEAVY_CMD_ID`, then recorded; the waiting line. |
| `src/logs.rs` | The `runs` record: writing, forgetting after 30 days, reading it read-only, and `logs`' lines. |
| `src/queue.rs` | `Heavy`: the queue in `heavy.db`, admission, liveness, status, the hourly update check's turn; where `heavy.db` lives. |
| `src/update.rs` | The self-update: the newest tag, `SHA256SUMS`, the verified download and switch, the detached hourly start. |
| `src/sha256.rs` | SHA-256, to check a downloaded binary against `SHA256SUMS`. |
| `src/cpu.rs` | The machine's CPU usage over the last 10 seconds, from samples kept in `heavy.db`. |
| `src/format.rs` | Now in ms, durations, stdout and stderr lines. |
| `src/sys/` | What differs between systems: `unix.rs` (Linux) and `windows.rs` — CPU counters, boot time, pid liveness, the shell, signals or console events, the job object, the update's detached start and binary switch. |
| `install.sh`, `install.ps1` | Install the newest release, its SHA-256 checked: `HEAVY_CMD_BIN_DIR` on Linux, `HEAVY_CMD_INSTALL_DIR` on Windows. |
| `scripts/Dockerfile`, `scripts/dev` | The toolchain, and the command that runs anything in it. |
| `scripts/check` | Every check: format, clippy and the suites, for Linux and Windows. |
| `scripts/pack` | Builds both release binaries, with `self-update`, and their `SHA256SUMS` into `dist/`, and checks each runs. |
| `scripts/release` | Publishes a release from the machine: check, pack, tag, `gh release create` with the three assets. |
| `tests/` | The suites (`common/` their fakes); `line_budget.rs` keeps every `.rs` file to 450 lines. |

## Tests

`heavy-cmd -- scripts/check` — `cargo fmt --check`, clippy with warnings denied for both targets, then every suite for Linux and for Windows under Wine — is the floor for every change. The suites:

- `queue.rs`, `cpu.rs`: admission, the gap, the CPU target, liveness, the waiting line, on a fake clock, fake CPU counters and fake pids (`common::World`), each waiter a thread with its own connection.
- `logs.rs`: the record of each outcome, forgetting after 30 days, and `logs`' lines.
- `update.rs`: SHA-256 against the standard digests (and `sha256sum` on Linux), the tag from the redirect, `SHA256SUMS`' lines, the update against fake releases — a newer one switched, the same or older left alone, a sum that does not match or a binary that names another release never put in place — and the hourly turn on a fake clock.
- `cli.rs`: `heavy-cmd` as a process — the usage and 64, exits, nesting, a signal while waiting (Linux), `PATHEXT` (Windows), a test build's refused `update`, and the smoke test: two commands in a temp home, the second waiting out the 10 s gap, and `logs` showing both.

Tests never touch the real queue nor GitHub: each runs on a temp state folder (`common::heavy_cmd` sets `XDG_STATE_HOME`, `LOCALAPPDATA` on Windows), with `HEAVY_CMD_ID` cleared.

## Workflow

0. Docker, and nothing else. `scripts/dev` builds the toolchain image the first time (and when `scripts/Dockerfile` changes).
1. Edit `src/`; `scripts/dev cargo build`, then `target/debug/heavy-cmd` runs on the machine as it is (point `XDG_STATE_HOME` at a temp folder when trying it by hand, so it stays off the machine's queue). `scripts/dev cargo test --test queue` runs one suite; add `--target x86_64-pc-windows-gnu` for Windows'.
2. `heavy-cmd -- scripts/check` before a change counts as done.

## Cutting a release

1. Bump `version` in `Cargo.toml` (then `scripts/dev cargo build` to update `Cargo.lock`), Commit, push `main`.
2. `heavy-cmd -- scripts/release`: it refuses a dirty or unpushed `main` or an existing tag, runs `scripts/check` and `scripts/pack`, pushes tag `v<version>` and creates the GitHub release with both binaries and `SHA256SUMS`.
3. Check it: `gh release view v<x.y.z>` lists the three assets; `heavy-cmd update` on an install moves to it.

Every install moves to a release within an hour of its next command: there is no CI, so `scripts/check` is the only gate. Never move or reuse a published tag: installs only go forward.
