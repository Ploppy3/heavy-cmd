use crate::cpu::{SAMPLE_MS, usage_line};
use crate::fail::{Fail, Result};
use crate::format::{duration, warn};
use crate::logs::{GAVE_UP, RAN, Run, STOPPED};
use crate::queue::{ClaimOptions, Heavy, HeavyStatus};
use crate::sys;
use std::time::Duration;

/// What heavy-cmd says on stderr, once, while it waits to start.
pub const WAITING: &str = "every heavy slot is busy";

/// Set for the command, to its id in the queue: a heavy-cmd it starts runs at once instead of waiting on itself.
pub const INSIDE: &str = "HEAVY_CMD_ID";

/// How often a running command's heavy-cmd looks whether it ended.
const POLL_CHILD: Duration = Duration::from_millis(50);

/// `heavy-cmd: every heavy slot is busy: 1 running · cpu 63%/60%, last start 12s ago; waiting up to 900s`.
pub fn waiting_line(status: &HeavyStatus, target: u32, wait_sec: u64, now: i64) -> String {
    let last = match status.last_start {
        Some(at) => format!("last start {} ago", duration(now - at)),
        None => "no start yet".to_string(),
    };
    let usage = usage_line(status.running.len(), status.cpu, target);
    format!("heavy-cmd: {WAITING}: {usage}, {last}; waiting up to {wait_sec}s")
}

/// Runs `argv` once it may start, or at once `inside` another heavy command, holding its start until it
/// ends; then records the run, and returns its exit code.
pub fn run_heavy(heavy: &Heavy, argv: &[String], options: &mut ClaimOptions, inside: bool) -> Result<i32> {
    let queued_at = heavy.now();
    let claimed = if inside { Ok(None) } else { heavy.claim(options, std::process::id()).map(Some) };
    let cwd = std::env::current_dir().map(|dir| dir.display().to_string()).unwrap_or_default();
    let mut run = Run { command: argv.join(" "), cwd, queued_at, started_at: None, ended_at: 0, exit: None, outcome: RAN };
    let ran = match claimed {
        Ok(id) => {
            run.started_at = Some(heavy.now());
            let code = run_command(argv, id.map(|id| (heavy, id)));
            if let Some(id) = id {
                let _ = heavy.release(id);
            }
            run.exit = Some(i64::from(code));
            Ok(code)
        }
        Err(fail) => {
            run.exit = Some(i64::from(fail.code()));
            run.outcome = if matches!(fail, Fail::Stopped(_)) { STOPPED } else { GAVE_UP };
            Err(fail)
        }
    };
    run.ended_at = heavy.now();
    heavy.record(&run);
    ran
}

/// Runs the command in the foreground, its stdio this process's, and returns its exit code: 127 when it
/// cannot start. One argument is a shell line (`sh -c`, `cmd.exe /c` on Windows), several are the program
/// and its arguments. With its start in `heavy`, the command gets its id as `HEAVY_CMD_ID`, and keeps the
/// CPU reading fresh for the waiters while it runs; without, it runs inside another heavy command.
fn run_command(argv: &[String], start: Option<(&Heavy, i64)>) -> i32 {
    let (mut command, name) = match argv {
        [line] => (sys::shell(line), sys::SHELL),
        [file, args @ ..] => (sys::program(file, args), file.as_str()),
        [] => return 127,
    };
    if let Some((_, id)) = start {
        command.env(INSIDE, id.to_string());
    }
    let mut child = match command.spawn() {
        Ok(child) => child,
        Err(error) => {
            warn(&format!("heavy-cmd: cannot run {name}: {error}"));
            return 127;
        }
    };
    let _running = sys::running(&child);
    let mut sampled_at = start.map_or(0, |(heavy, _)| heavy.now());
    loop {
        match child.try_wait() {
            Ok(Some(status)) => return sys::exit_of(status),
            Ok(None) => {
                // A sample every 2 seconds; one that fails is taken again at the next.
                if let Some((heavy, _)) = start
                    && heavy.now() - sampled_at >= SAMPLE_MS
                {
                    sampled_at = heavy.now();
                    let _ = heavy.sample();
                }
                std::thread::sleep(POLL_CHILD);
            }
            Err(error) => {
                warn(&format!("heavy-cmd: cannot wait for {name}: {error}"));
                return 1;
            }
        }
    }
}
