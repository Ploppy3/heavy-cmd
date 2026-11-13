use heavy_cmd::fail::{EX_USAGE, Fail, Result};
use heavy_cmd::format::{now_ms, say, warn};
use heavy_cmd::logs::{LIMIT, clock, line, recent};
use heavy_cmd::queue::{CPU_TARGET, ClaimOptions, Heavy, HeavyPorts, WAIT_SEC, state_dir};
use heavy_cmd::run::{INSIDE, run_heavy, waiting_line};
use heavy_cmd::sys;

const USAGE: &str = concat!(
    "heavy-cmd v",
    env!("CARGO_PKG_VERSION"),
    ": run a heavy command (an install, a build, a test suite) once the machine has room

  heavy-cmd -- <command>...
      run it once it is the machine's oldest waiter, 10s after the last start and, while others run,
      with the CPU over the last 10s under 60%; one argument runs through sh -c (cmd.exe /c on
      Windows); exits with the command's code, or 75 when it could not start within 900s
  heavy-cmd logs
      the last 50 runs of the last 30 days, the last ended first"
);

/// Runs `command` once it may start, inside another heavy command at once, and returns its exit.
fn run(command: &[String]) -> Result<i32> {
    let inside = std::env::var_os(INSIDE).is_some_and(|id| !id.is_empty());
    let heavy = Heavy::open(&state_dir(), HeavyPorts::machine())?;
    let on_wait = |status: &_| warn(&waiting_line(status, CPU_TARGET, WAIT_SEC, now_ms()));
    let mut options = ClaimOptions { on_wait: Some(Box::new(on_wait)), stop: Some(&sys::STOP), ..ClaimOptions::default() };
    run_heavy(&heavy, command, &mut options, inside)
}

/// `heavy-cmd logs`: the recorded runs, one line each, when it ended first.
fn logs() -> Result<i32> {
    let runs = recent(&state_dir(), LIMIT)?;
    if runs.is_empty() {
        say("no heavy command recorded");
    }
    let (now, home) = (now_ms(), std::env::home_dir());
    for run in &runs {
        say(&format!("{}  {}", clock(run.ended_at, now), line(run, home.as_deref())));
    }
    Ok(0)
}

fn main() {
    let argv: Vec<String> = std::env::args_os().skip(1).map(|arg| arg.to_string_lossy().into_owned()).collect();
    let ran = match argv.split_first() {
        Some((first, command)) if first == "--" && !command.is_empty() => {
            let signals = sys::catch_signals();
            let ran = run(command);
            drop(signals);
            ran
        }
        Some((first, [])) if first == "logs" => logs(),
        _ => {
            warn(USAGE);
            std::process::exit(EX_USAGE);
        }
    };
    let code = ran.unwrap_or_else(|fail| {
        if let Fail::Stopped(signal) = fail {
            sys::die_by(signal);
        }
        warn(&format!("heavy-cmd: {fail}"));
        fail.code()
    });
    std::process::exit(code);
}
