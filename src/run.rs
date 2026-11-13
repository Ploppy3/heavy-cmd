use crate::cpu::SAMPLE_MS;
use crate::fail::{Fail, Result};
use crate::format::iso;
use crate::queue::{ClaimOptions, Heavy, Ran};
use crate::runs::{GAVE_UP, RAN, STOPPED};
use crate::sys;
use std::cell::Cell;
use std::time::Duration;

/// What heavy-cmd says on stderr while it waits to start: a stable text callers match.
pub const WAITING: &str = "every heavy slot is busy";

/// Set for the command, to its id in the queue: a heavy command it starts runs within its admission instead of waiting on itself.
pub const INSIDE: &str = "HEAVY_CMD_ID";

/// Who runs a command, when no `--agent` says it; handed on to the command, so one it starts is recorded as theirs too.
pub const AGENT: &str = "HEAVY_CMD_AGENT";

/// How often a running command's heavy-cmd looks whether it ended.
const POLL_CHILD: Duration = Duration::from_millis(50);

/// Work admitted to run: the env a command it starts gets, and the samples it keeps while it runs.
pub struct Admitted<'h> {
    heavy: &'h Heavy,
    /// Its id in the queue; None inside another heavy command.
    id: Option<i64>,
    agent: Option<String>,
    sampled_at: Cell<i64>,
}

impl Admitted<'_> {
    /// What a command it starts gets on top of this process's env: its queue id, when it has one, and its agent.
    pub fn env(&self) -> Vec<(&'static str, String)> {
        let mut env = Vec::new();
        if let Some(id) = self.id {
            env.push((INSIDE, id.to_string()));
        }
        if let Some(agent) = &self.agent {
            env.push((AGENT, agent.clone()));
        }
        env
    }

    /// While it runs, its samples keep the machine's CPU reading fresh for the waiters, every 2 seconds;
    /// one that fails is taken again at the next.
    pub fn tick(&self) {
        let now = self.heavy.now();
        if self.id.is_some() && now - self.sampled_at.get() >= SAMPLE_MS {
            self.sampled_at.set(now);
            let _ = self.heavy.sample();
        }
    }
}

/// What `work` returns, recorded once it is over: `exit_of` its value, 1 when it fails.
fn recorded<T>(work: impl FnOnce() -> Result<T>, exit_of: impl Fn(&T) -> i64, record: impl FnOnce(i64)) -> Result<T> {
    let value = work();
    record(value.as_ref().map_or(1, exit_of));
    value
}

/// Runs `work` once a heavy command may start, holding its slot until `work` returns. Inside another
/// heavy command it runs straight away, since waiting on itself never ends. Once over it is recorded:
/// run, with `exit_of` its value as its exit or 1 when it fails; given up, with the code heavy-cmd
/// exits with; or stopped while it waited.
pub fn heavily<T>(
    heavy: &Heavy,
    options: &mut ClaimOptions,
    work: impl FnOnce(&Admitted) -> Result<T>,
    exit_of: impl Fn(&T) -> i64,
) -> Result<T> {
    let queued_at = iso(heavy.now());
    let agent = options.agent.clone().filter(|agent| !agent.is_empty());
    if let Some(inside) = options.inside.clone() {
        let id = inside.bytes().all(|b| b.is_ascii_digit()).then(|| inside.parse().ok()).flatten();
        let admitted = Admitted { heavy, id: None, agent, sampled_at: Cell::new(0) };
        let ran = |exit| Ran {
            started_at: Some(queued_at.clone()),
            exit: Some(exit),
            outcome: RAN,
            inside: id,
            queued_at: queued_at.clone(),
            ..Ran::default()
        };
        return recorded(|| work(&admitted), exit_of, |exit| heavy.record(options, ran(exit)));
    }
    let holder = match heavy.claim(options, heavy.pid()) {
        Ok(holder) => holder,
        Err(fail) => {
            let (exit, outcome) = match &fail {
                Fail::Stopped(signal) => (signal.map(|signal| 128 + i64::from(signal)), STOPPED),
                fail => (Some(i64::from(fail.code())), GAVE_UP),
            };
            heavy.record(options, Ran { queued_at, started_at: None, exit, outcome, ..Ran::default() });
            return Err(fail);
        }
    };
    let id = holder.who.id;
    let admitted = Admitted { heavy, id: Some(id), agent, sampled_at: Cell::new(heavy.now()) };
    let ran = |exit| Ran {
        queued_at,
        started_at: Some(holder.started_at.clone()),
        exit: Some(exit),
        outcome: RAN,
        heavy_id: Some(id),
        inside: None,
    };
    let value = recorded(|| work(&admitted), exit_of, |exit| heavy.record(options, ran(exit)));
    let _ = heavy.release(id);
    value
}

/// Runs `argv` once it may start, and returns its exit code: one argument is a shell line (`sh -c`,
/// `cmd.exe /c` on Windows), several are the program and its arguments.
pub fn run_heavy(heavy: &Heavy, argv: &[String], options: &mut ClaimOptions) -> Result<i32> {
    options.command = Some(argv.to_vec());
    heavily(heavy, options, |admitted| Ok(run_command(argv, admitted)), |code| i64::from(*code))
}

/// Runs the command in the foreground, its stdio this process's, and returns its exit code: 127 when it cannot start.
fn run_command(argv: &[String], admitted: &Admitted) -> i32 {
    let (mut command, name) = match argv {
        [line] => (sys::shell(line), sys::SHELL),
        [file, args @ ..] => (sys::program(file, args), file.as_str()),
        [] => return 127,
    };
    command.envs(admitted.env());
    let mut child = match command.spawn() {
        Ok(child) => child,
        Err(error) => {
            admitted.heavy.warn(&format!("heavy-cmd: cannot run {name}: {error}"));
            return 127;
        }
    };
    let _running = sys::running(&child);
    loop {
        match child.try_wait() {
            Ok(Some(status)) => return sys::exit_of(status),
            Ok(None) => {
                admitted.tick();
                std::thread::sleep(POLL_CHILD);
            }
            Err(error) => {
                admitted.heavy.warn(&format!("heavy-cmd: cannot wait for {name}: {error}"));
                return 1;
            }
        }
    }
}
