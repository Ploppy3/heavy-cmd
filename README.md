# heavy-cmd

Run a heavy command — an install, a build, a test suite — once the machine has room. Every heavy-cmd on the machine, whichever agent, project or terminal starts it, joins one machine-wide queue, so a dozen agents running `npm ci` at once take turns instead of drowning the machine.

```sh
heavy-cmd -- npm run check
```

## Install

One binary, Linux and Windows on x86_64. Run the installer again to update.

**Linux** (needs `curl`): installs `~/.local/bin/heavy-cmd` (`HEAVY_CMD_BIN_DIR` overrides the folder).

```sh
curl -fsSL https://raw.githubusercontent.com/Ploppy3/heavy-cmd/main/install.sh | sh
```

**Windows** (PowerShell): installs `%LOCALAPPDATA%\Programs\heavy-cmd\heavy-cmd.exe` (`HEAVY_CMD_INSTALL_DIR` overrides the folder) and puts that folder on your PATH.

```powershell
irm https://raw.githubusercontent.com/Ploppy3/heavy-cmd/main/install.ps1 | iex
```

## Usage

```
heavy-cmd -- <command>...
```

That is the only command line it takes: anything else prints the usage, whose first line names the version, and exits `64`.

It waits for its turn, then runs the command in the foreground: stdin, stdout and stderr are the command's, and heavy-cmd exits with the command's exit code. One argument is a shell line, run through `sh -c` (`heavy-cmd -- 'npm ci && npm test'`), or `cmd.exe /d /s /c` on Windows; several are the program and its arguments. On Windows a bare program name is looked up with `PATHEXT`, so `heavy-cmd -- npm ci` finds `npm.cmd`.

A command starts when:

- it is the oldest waiter on the machine (one FIFO queue);
- the last start was at least 10 seconds ago, long enough for it to show in the CPU usage;
- and, while other heavy commands run, the whole machine's CPU usage over the last 10 seconds is under 60 %. With none running, the gap alone lets it in, whatever the CPU.

It checks every 2 seconds, and gives up with exit `75` when it could not start within 900 seconds.

A signal (`SIGINT`, `SIGTERM`, `SIGHUP`) while it waits stops the wait and ends heavy-cmd as that signal would end the command. Once the command runs, `SIGTERM` and `SIGHUP` are passed on to it, and a terminal's `SIGINT` reaches it through its process group. On Windows, Ctrl+C, Ctrl+Break or the console closing while it waits exit `130`, `149` or `129`; once the command runs, the console's events reach it directly, and it runs in a job object, so whatever ends heavy-cmd ends it too.

The queue lives in `heavy.db` (SQLite) in `${XDG_STATE_HOME:-~/.local/state}/heavy-cmd`, `%LOCALAPPDATA%\heavy-cmd` on Windows.

## Exit codes

| Exit | Meaning |
|------|---------|
| the command's | it ran: its own exit code, `128 + n` when signal `n` ended it; a shell line's unknown program is the shell's to report (`127` from `sh`, `9009` from `cmd.exe`) |
| `75` | it could not start within 900 s (sysexits' `EX_TEMPFAIL`); the command never ran |
| `127` | the program could not be started (not found, not executable) |
| `64` | a command line other than `heavy-cmd -- <command>...` (sysexits' `EX_USAGE`) |
| `1` | another failure of heavy-cmd's own, such as a `heavy.db` it cannot open |

## Output

- **The waiting line.** When a command cannot start on its first try, heavy-cmd writes one line on stderr, once, containing `every heavy slot is busy`:
  ```
  heavy-cmd: every heavy slot is busy: 1 running · cpu 72%/60%, last start 3s ago; waiting up to 900s
  ```
  Match the phrase, not the rest of the line.
- **Exit 75** means the command never ran, with one stderr line, `heavy-cmd: no heavy command could start in 900s`.
- **Nesting.** The command runs with `HEAVY_CMD_ID` set to its id in the queue. A heavy-cmd started with `HEAVY_CMD_ID` set runs its command at once, since waiting for its own parent would never end.
- Its own messages on stderr start with `heavy-cmd: `.

## Development

Rust, built and tested in a Docker toolchain so nothing but Docker is needed: `scripts/check` runs the format, clippy and every suite for Linux and for Windows (under Wine). See [AGENTS.md](AGENTS.md).

## License

MIT, see [LICENSE](LICENSE).
