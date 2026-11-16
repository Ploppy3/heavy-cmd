use heavy_cmd::fail::{EX_USAGE, Fail, Result};
use heavy_cmd::format::{now_ms, say, warn};
use heavy_cmd::logs::{LIMIT, clock, line, recent};
use heavy_cmd::queue::{CPU_TARGET, ClaimOptions, Heavy, HeavyPorts, WAIT_SEC, state_dir};
use heavy_cmd::run::{INSIDE, run_heavy, waiting_line};
use heavy_cmd::sys;
use heavy_cmd::update::{SELF_UPDATE, UpdatePorts, auto_update, update};
use heavy_cmd::watch::watch;

const USAGE: &str = concat!(
    "heavy-cmd v",
    env!("CARGO_PKG_VERSION"),
    ": run a heavy command (an install, a build, a test suite) once the machine has room

  heavy-cmd
      on a terminal, the queue live until Ctrl+C: the commands running, the next ones and the last 10 runs
  heavy-cmd -- <command>...
      run it once it is the machine's oldest waiter, 10s after the last start and, while others run,
      with the CPU over the last 10s under 60%; one argument runs through sh -c (cmd.exe /c on
      Windows); exits with the command's code, or 75 when it could not start within 900s
  heavy-cmd logs
      the last 50 runs of the last 30 days, the last ended first
  heavy-cmd update
      move to the newest release now, once its SHA-256 matches the release's SHA256SUMS; a release's
      heavy-cmd also does it in the background, at most once an hour, after a command ends"
);

/// Runs `command` once it may start, inside another heavy command at once, and returns its exit; then,
/// unless a signal stopped it, the hourly update check.
fn run(command: &[String]) -> Result<i32> {
    let inside = std::env::var_os(INSIDE).is_some_and(|id| !id.is_empty());
    let state = state_dir();
    let heavy = Heavy::open(&state, HeavyPorts::machine())?;
    let on_wait = |status: &_| warn(&waiting_line(status, CPU_TARGET, WAIT_SEC, now_ms()));
    let mut options = ClaimOptions { on_wait: Some(Box::new(on_wait)), stop: Some(&sys::STOP), ..ClaimOptions::default() };
    let ran = run_heavy(&heavy, command, &mut options, inside);
    if !matches!(ran, Err(Fail::Stopped(_))) {
        auto_update(&heavy, &state);
    }
    ran
}

/// `heavy-cmd update`: this binary moved to the newest release, when it is a release's.
fn self_update() -> Result<i32> {
    if !SELF_UPDATE {
        return Err(Fail::Other("this build does not update itself: only a release's binary does".into()));
    }
    update(&UpdatePorts::github(), env!("CARGO_PKG_VERSION"), &std::env::current_exe()?, &mut |text| say(text))?;
    Ok(0)
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

/// The usage, on stderr, and exit 64.
fn usage() -> ! {
    warn(USAGE);
    std::process::exit(EX_USAGE)
}

fn main() {
    let argv: Vec<String> = std::env::args_os().skip(1).map(|arg| arg.to_string_lossy().into_owned()).collect();
    let ran = match argv.split_first() {
        // Off a terminal it is the usage: a heavy-cmd checks the release it updates to by its usage's first line.
        None => match sys::term_size() {
            Some(size) => Heavy::open(&state_dir(), HeavyPorts::machine()).and_then(|heavy| watch(&heavy, size)).map(|()| 0),
            None => usage(),
        },
        Some((first, command)) if first == "--" && !command.is_empty() => {
            let signals = sys::catch_signals();
            let ran = run(command);
            drop(signals);
            ran
        }
        Some((first, [])) if first == "logs" => logs(),
        Some((first, [])) if first == "update" => self_update(),
        _ => usage(),
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
