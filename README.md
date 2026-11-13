# heavy-cmd

Run a heavy command — an install, a build, a test suite — once the machine has room. Every heavy-cmd on the machine, whichever agent, project or terminal starts it, joins one machine-wide queue, so a dozen agents running `npm ci` at once take turns instead of drowning the machine.

```sh
heavy-cmd -- npm run check
```

## Install

One binary, nothing to install beside it: Linux and Windows, on x86_64.

**Linux** (needs `curl` and `tar`):

```sh
curl -fsSL https://raw.githubusercontent.com/Ploppy3/heavy-cmd/main/install.sh | sh
```

It puts the newest [release](https://github.com/Ploppy3/heavy-cmd/releases) in `~/.local/share/heavy-cmd/bin/heavy-cmd` and links `~/.local/bin/heavy-cmd` to it. `HEAVY_CMD_INSTALL_DIR` and `HEAVY_CMD_BIN_DIR` override the two folders. To remove it: `rm -rf ~/.local/share/heavy-cmd ~/.local/bin/heavy-cmd`, and `~/.local/state/heavy-cmd` for its record.

**Windows** (PowerShell; Windows 10 1803 or later, for its `curl.exe` and `tar.exe`):

```powershell
irm https://raw.githubusercontent.com/Ploppy3/heavy-cmd/main/install.ps1 | iex
```

It puts the newest release in `%LOCALAPPDATA%\Programs\heavy-cmd\bin\heavy-cmd.exe` and that `bin` folder on your PATH; `HEAVY_CMD_INSTALL_DIR` overrides the folder. To remove it: delete that folder, its PATH entry, and `%LOCALAPPDATA%\heavy-cmd` for its record.

Run either again at will: it changes nothing when the newest release is already there. An install from the Node releases (v0.1) is replaced in place by `install.sh`.

## Usage

```
heavy-cmd [--label <text>] [--project <name>] [--wait <s>] [--agent <name>] -- <command>...
heavy-cmd status [--json]
heavy-cmd logs [--project <name>] [--limit <n>] [--since <ISO time | 1h>] [--json]
heavy-cmd update [--check]
heavy-cmd --version | --help
```

### Running a command

`heavy-cmd -- <command>` waits for its turn, then runs the command in the foreground: stdin, stdout and stderr are the command's, and heavy-cmd exits with the command's exit code. One argument is a shell line, run through `sh -c` (`heavy-cmd -- 'npm ci && npm test'`), or `cmd.exe /d /s /c` on Windows; several are the program and its arguments. On Windows a bare program name is looked up with `PATHEXT`, so `heavy-cmd -- npm ci` finds `npm.cmd`.

A command starts when:

- it is the oldest waiter on the machine (one FIFO queue);
- the last start was at least 10 seconds ago, long enough for it to show in the CPU usage;
- and, while other heavy commands run, the whole machine's CPU usage over the last 10 seconds is under the target (`cpu`, 60 % unless set). With none running, the gap alone lets it in, whatever the CPU.

It checks every 2 seconds, and gives up with exit `75` when it could not start within `--wait` seconds (`wait`, 900 unless set).

- `--label` names it in `status` and `logs`: the command's first 80 characters unless given.
- `--project` is the project it is recorded under. Unless given: the repository name of the git `origin` remote of the working directory, else the git top-level folder's name, else the working directory's name. Origin comes first because a worktree's folder name is not its project's.
- `--agent` is who ran it, else `HEAVY_CMD_AGENT`; none when neither is set.

A signal (`SIGINT`, `SIGTERM`, `SIGHUP`) while it waits stops the wait — recorded as stopped — and ends heavy-cmd as that signal would end the command. Once the command runs, `SIGTERM` and `SIGHUP` are passed on to it, and a terminal's `SIGINT` reaches it through its process group. A signal heavy-cmd was started ignoring, as under `nohup`, stays ignored.

On Windows the console's events stand for the signals: Ctrl+C, Ctrl+Break or the console closing while it waits stop the wait, recorded as stopped, and heavy-cmd exits `130`, `149` or `129` (`128 +` the C runtime's `SIGINT`, `SIGBREAK`, and `SIGHUP` for a closing). Once the command runs, the console's events reach it directly, and heavy-cmd exits as it does. The command runs in a job object, so whatever ends heavy-cmd ends the command too; once the command is over, what it left running lives on.

### `heavy-cmd status`

The commands running, the waiters in the order they start, the CPU usage against its target, and the time since the last start:

```
1 running · cpu 72%/60%, last start 42s ago
running  api: npm test (pid 4242, since 2026-11-12T10:00:00.000Z)
waiting  1. web: npm ci (pid 4300, since 2026-11-12T10:00:30.000Z)
```

`--json` gives `running` (`id`, `pid`, `host`, `project`, `label`, `started_at`), `waiting` (the same, with `since`), `cpu` (null until measured), `last_start`, `target` and `since_last_start_sec`.

### `heavy-cmd logs`

Every run is recorded once it is over, and kept 30 days. `logs` lists them, the last ended first, 50 unless `--limit`; `--project` keeps one project's, `--since` those ended since an ISO time or a duration back from now (`90s`, `15m`, `1h30m`, `2d`).

```
14:02:11  api  npm run check  exit 0  waited 12s  ran 3m 4s  task-73
13:58:40  web  npm ci  gave up after 15m (75)
```

`--json` gives each run whole: `id`, `project`, `label`, `command` (its argv; one element for a shell line), `cwd`, `host`, `pid`, `agent`, `heavy_id` (its id in the queue), `inside` (the id of the heavy-cmd it ran inside), `queued_at`, `started_at` (null when it never started), `ended_at`, `exit` and `outcome`: `ran`, `gave up` or `stopped while waiting`.

## Configuration

`${XDG_CONFIG_HOME:-~/.config}/heavy-cmd/config` (`%APPDATA%\heavy-cmd\config` on Windows), one `key = value` per line, `#` starting a comment:

```
cpu = 60          # the CPU target, in %: 10 to 100
wait = 900        # how long a command waits to start, in seconds: 0 to 86400
auto_update = on  # on or off
```

The env overrides the file — `HEAVY_CMD_CPU`, `HEAVY_CMD_WAIT`, `HEAVY_CMD_AUTO_UPDATE` (empty is unset) — and `--wait` overrides the env. A value out of its bounds, an unknown key or a line that is not `key = value` is refused, naming where it is and why, and nothing runs (exit `78`).

The queue and its record, `heavy.db` (SQLite), live in `$HEAVY_CMD_HOME`, else `${XDG_STATE_HOME:-~/.local/state}/heavy-cmd` (`%LOCALAPPDATA%\heavy-cmd` on Windows). Everyone who should share a queue must share this folder: by default, every heavy-cmd of one user. The Node releases (v0.1) kept the same file, the same way: both can share one queue.

## Exit codes

| Exit | Meaning |
|------|---------|
| the command's | it ran: its own exit code, `128 + n` when signal `n` ended it; a shell line's unknown program is the shell's to report (`127` from `sh`, `9009` from `cmd.exe`) |
| `75` | it could not start within the wait (sysexits' `EX_TEMPFAIL`); the command never ran |
| `127` | the program could not be started (not found, not executable) |
| `64` | a command line heavy-cmd does not take (sysexits' `EX_USAGE`) |
| `78` | a setting out of its bounds (sysexits' `EX_CONFIG`) |
| `1` | another failure of heavy-cmd's own, such as an update that failed |

A wait stopped by a signal ends heavy-cmd by that signal, as a shell reports it: `128 + n`; on Windows it exits `128 + n` (`130` for Ctrl+C).

## Output contract

Callers — agents, scripts, the factory — may rely on these; they change only with a major version.

- **The waiting line.** When a command cannot start on its first try, heavy-cmd writes one line on stderr, once, containing `every heavy slot is busy`:
  ```
  heavy-cmd: every heavy slot is busy: 1 running · cpu 72%/60%, last start 3s ago (api: npm test); waiting up to 900s
  ```
  Match the phrase, not the rest of the line. With `--wait 0` it gives up at once, without the line.
- **Exit 75** means the command never ran because no start came within the wait, with one stderr line, `heavy-cmd: no heavy command could start in <n>s` (`... may start now` with `--wait 0`). It is never the command's own code unless the command itself exits 75.
- **`HEAVY_CMD_AGENT`** names who runs the command when no `--agent` does, and is recorded with its run. With `--agent`, the command gets `HEAVY_CMD_AGENT` set to it, so the heavy-cmds it starts are recorded under the same agent.
- **Nesting.** The command runs with `HEAVY_CMD_ID` set to its id in the queue. A heavy-cmd that starts with `HEAVY_CMD_ID` set runs its command at once — no gap, no CPU check, no wait — since waiting for its own parent would never end, and is recorded with `inside` set to that id.
- Its own messages on stderr start with `heavy-cmd: `. A run that cannot be recorded says so in one such line, and its exit stays the command's.

## Updating

- `heavy-cmd update` moves the install to the newest release, never back to an older one. It downloads this system's asset with `curl` and unpacks it with `tar` (Windows' own `curl.exe` and `tar.exe`) beside the binary in use, and only once the new one runs (`--version`) puts it in its place, keeping the one before as `bin/heavy-cmd.old` (`heavy-cmd.old.exe`). On Linux that is one atomic rename; on Windows, where a running program can be renamed but not replaced, the one in use is renamed away first. On any failure the install stays as it was. `heavy-cmd update --check` only says whether a newer release is out.
- **Auto update:** after a command ends, at most once a day, heavy-cmd starts `heavy-cmd update` detached in the background, its output in `update.log` in the install folder. It never delays the command, nor changes its output or exit. `auto_update = off` (or `HEAVY_CMD_AUTO_UPDATE=off`) turns it off.
- An install is a `bin/heavy-cmd` beside a `.heavy-cmd-install` file, as the installers leave it. A dev build — one under a git clone with its `Cargo.toml` — is updated with git: `update` refuses there, and it never updates itself; nor does a binary copied anywhere else.
- `HEAVY_CMD_RELEASES_API` points `update` and the installers at another releases API than GitHub's (a mirror, or a test's server); it must answer as GitHub's `releases/latest`.

## Development

Rust, built and tested in a Docker toolchain so nothing but Docker is needed: `scripts/check` runs the format, clippy and every suite for Linux and for Windows (under Wine). See [AGENTS.md](AGENTS.md).

## License

MIT, see [LICENSE](LICENSE).
