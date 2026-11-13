use heavy_cmd::fail::{EX_USAGE, Fail, Result};
use heavy_cmd::format::{now_ms, warn};
use heavy_cmd::queue::{CPU_TARGET, ClaimOptions, Heavy, HeavyPorts, WAIT_SEC, state_dir};
use heavy_cmd::run::{INSIDE, run_command, run_heavy, waiting_line};
use heavy_cmd::sys;

const USAGE: &str = concat!(
    "heavy-cmd v",
    env!("CARGO_PKG_VERSION"),
    ": run a heavy command (an install, a build, a test suite) once the machine has room

  heavy-cmd -- <command>...

It starts once it is the machine's oldest waiter, 10s after the last start and, while others run, with
the CPU over the last 10s under 60%; one argument runs through sh -c (cmd.exe /c on Windows). It exits
with the command's code, or 75 when it could not start within 900s."
);

/// Runs `command` once it may start, inside another heavy command at once, and returns its exit.
fn run(command: &[String]) -> Result<i32> {
    if std::env::var_os(INSIDE).is_some_and(|id| !id.is_empty()) {
        return Ok(run_command(command, None));
    }
    let heavy = Heavy::open(&state_dir(), HeavyPorts::machine())?;
    let on_wait = |status: &_| warn(&waiting_line(status, CPU_TARGET, WAIT_SEC, now_ms()));
    let mut options = ClaimOptions { on_wait: Some(Box::new(on_wait)), stop: Some(&sys::STOP), ..ClaimOptions::default() };
    run_heavy(&heavy, command, &mut options)
}

fn main() {
    let argv: Vec<String> = std::env::args_os().skip(1).map(|arg| arg.to_string_lossy().into_owned()).collect();
    let Some(("--", command)) = argv.split_first().map(|(first, rest)| (first.as_str(), rest)).filter(|(_, rest)| !rest.is_empty()) else {
        warn(USAGE);
        std::process::exit(EX_USAGE);
    };
    let signals = sys::catch_signals();
    let ran = run(command);
    drop(signals);
    let code = ran.unwrap_or_else(|fail| {
        if let Fail::Stopped(signal) = fail {
            sys::die_by(signal);
        }
        warn(&format!("heavy-cmd: {fail}"));
        fail.code()
    });
    std::process::exit(code);
}
