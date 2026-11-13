use heavy_cmd::cli::{Parsed, parse};
use heavy_cmd::commands::{cmd_logs, cmd_run, cmd_status};
use heavy_cmd::config::Env;
use heavy_cmd::fail::{EX_USAGE, Fail, Result};
use heavy_cmd::format::{now_ms, say, warn};
use heavy_cmd::install::{VERSION, install_of, this_exe, version_line};
use heavy_cmd::sys;
use heavy_cmd::update::{auto_update, github_ports, start_detached, update};

const USAGE: &str = "heavy-cmd: run a heavy command (an install, a build, a test suite) once the machine has room

  heavy-cmd [--label <text>] [--project <name>] [--wait <s>] [--agent <name>] -- <command>...
      run it once it may start: the machine's oldest waiter first, 10s after the last start and,
      while others run, with the CPU over the last 10s under the target (cpu, 60% unless set);
      one argument runs through sh -c (cmd.exe /c on Windows); exits with the command's code, or
      75 when it could not start within --wait (wait, 900s unless set)
  heavy-cmd status [--json]
      the commands running, the waiters in order, the CPU against its target, the last start
  heavy-cmd logs [--project <name>] [--limit <n>] [--since <ISO time | 1h>] [--json]
      the recorded runs of the last 30 days, the last ended first, 50 unless --limit
  heavy-cmd update [--check]
      move this install to the newest release; --check only says whether one is out
  heavy-cmd --version | --help

Config: ${XDG_CONFIG_HOME:-~/.config}/heavy-cmd/config (%APPDATA%\\heavy-cmd\\config on Windows):
cpu, wait, auto_update, overridden by HEAVY_CMD_CPU, HEAVY_CMD_WAIT, HEAVY_CMD_AUTO_UPDATE.
State: $HEAVY_CMD_HOME, else ${XDG_STATE_HOME:-~/.local/state}/heavy-cmd (%LOCALAPPDATA%\\heavy-cmd).";

const RUN_FLAGS: [&str; 4] = ["label", "project", "wait", "agent"];

/// Runs the command after `--`, then maybe starts the daily update; a wait stopped by a signal ends this
/// process by that signal, once recorded, before anything else.
fn run(command: &[String], parsed: &Parsed) -> Result<i32> {
    if !parsed.positionals.is_empty() {
        return Err(Fail::Usage(format!("\"{}\" before --: the command to run goes after --", parsed.positionals.join(" "))));
    }
    if command.is_empty() {
        return Err(Fail::Usage("nothing to run after --".into()));
    }
    parsed.flags.only("a command", &RUN_FLAGS)?;
    let env = Env::process();
    let cwd = std::env::current_dir()?;
    let signals = sys::catch_signals();
    let ran = cmd_run(command, &parsed.flags, &env, &cwd, Some(&sys::STOP));
    drop(signals);
    if let Err(Fail::Stopped(Some(signal))) = ran {
        sys::die_by(signal);
    }
    auto_update(&env, &install_of(&this_exe()), now_ms(), &mut start_detached);
    ran
}

/// Runs heavy-cmd with `argv`, and returns its exit code; a failure carries its own.
fn main_with(argv: &[String]) -> Result<i32> {
    let parsed = parse(argv)?;
    let flags = &parsed.flags;
    if flags.has("version") {
        say(&version_line(&install_of(&this_exe())));
        return Ok(0);
    }
    if flags.has("help") || parsed.positionals.first().is_some_and(|word| word == "help") {
        say(USAGE);
        return Ok(0);
    }
    if let Some(command) = &parsed.command {
        return run(command, &parsed);
    }
    let (sub, rest) = match parsed.positionals.split_first() {
        Some((sub, rest)) => (Some(sub.as_str()), rest),
        None => (None, &[][..]),
    };
    if let Some(sub) = sub.filter(|sub| !["status", "logs", "update"].contains(sub)) {
        let line = parsed.positionals.join(" ");
        return Err(Fail::Usage(format!("unknown command \"{sub}\": a command to run goes after --, as in heavy-cmd -- {line}")));
    }
    if let (Some(sub), false) = (sub, rest.is_empty()) {
        return Err(Fail::Usage(format!("{sub} takes no \"{}\"", rest.join(" "))));
    }
    let env = Env::process();
    match sub {
        Some("status") => {
            flags.only("status", &["json"])?;
            say(&cmd_status(flags, &env)?);
        }
        Some("logs") => {
            flags.only("logs", &["project", "limit", "since", "json"])?;
            say(&cmd_logs(flags, &env, now_ms())?);
        }
        Some(_) => {
            flags.only("update", &["check"])?;
            update(&github_ports(&env), flags.has("check"), &install_of(&this_exe()), &format!("v{VERSION}"), &mut |line| say(line))?;
        }
        None if RUN_FLAGS.iter().any(|key| flags.has(key)) => return Err(Fail::Usage("the command to run goes after --".into())),
        None => {
            warn(USAGE);
            return Ok(EX_USAGE);
        }
    }
    Ok(0)
}

fn main() {
    let argv: Vec<String> = std::env::args_os().skip(1).map(|arg| arg.to_string_lossy().into_owned()).collect();
    let code = main_with(&argv).unwrap_or_else(|fail| {
        warn(&format!("heavy-cmd: {fail}"));
        fail.code()
    });
    std::process::exit(code);
}
