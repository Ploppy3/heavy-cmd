//! `heavy-cmd` as a process: exits, the waiting line and 75, nesting, a signal while waiting, and the
//! smoke test: two commands in a temp home, the second waiting out the 10 s gap, and `logs` showing both.

mod common;

use common::{TempDir, heavy_cmd, line};
use heavy_cmd::fail::Result;
use heavy_cmd::install::VERSION;
use heavy_cmd::queue::{ClaimOptions, Heavy, HeavyPorts};
use heavy_cmd::serde_json::{self, Value};
use std::path::Path;
use std::process::Output;
use std::time::Instant;

fn cli(dir: &Path, args: &[&str], env: &[(&str, &str)]) -> Output {
    heavy_cmd(dir).args(args).envs(env.iter().copied()).output().unwrap()
}

fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).replace("\r\n", "\n")
}

fn logs(dir: &Path) -> Vec<Value> {
    serde_json::from_slice(&cli(dir, &["logs", "--json"], &[]).stdout).unwrap()
}

/// This process holds a start in `dir`'s queue, as `holder-project: holder`, until the Heavy drops.
fn hold(dir: &Path) -> Result<Heavy> {
    let heavy = Heavy::open(&dir.join("state"), HeavyPorts::machine())?;
    let mut options = ClaimOptions { project: "holder-project".into(), label: "holder".into(), ..ClaimOptions::default() };
    heavy.claim(&mut options, std::process::id())?;
    Ok(heavy)
}

#[test]
fn version_names_the_version_and_a_dev_checkout() {
    let dir = TempDir::new("heavy-cmd-cli");
    let out = cli(dir.path(), &["--version"], &[]);
    assert_eq!((out.status.code(), text(&out.stdout)), (Some(0), format!("heavy-cmd v{VERSION} (dev checkout)\n")));
}

#[test]
fn runs_the_command_in_the_foreground_its_output_its_exit_one_argument_is_a_shell_line() {
    let dir = TempDir::new("heavy-cmd-cli");
    // Each in a home of its own: a second start in one home waits out the gap.
    let home = |name: &str| dir.join(name).to_string_lossy().into_owned();
    let shell = line("echo out; echo err >&2; exit 3", "echo out& echo err 1>&2& exit 3");
    let out = cli(dir.path(), &["--", &shell], &[("HEAVY_CMD_HOME", &home("a"))]);
    assert_eq!((out.status.code(), text(&out.stdout).trim_end(), text(&out.stderr).trim_end()), (Some(3), "out", "err"));
    let id = line(r#"test -n "$HEAVY_CMD_ID" && echo "$HEAVY_CMD_ID""#, "echo %HEAVY_CMD_ID%");
    let out = cli(dir.path(), &["--", &id], &[("HEAVY_CMD_HOME", &home("b"))]);
    assert_eq!((out.status.code(), text(&out.stdout).trim_end()), (Some(0), "1"));
    // A shell line's unknown program is the shell's to report: sh exits 127, cmd.exe 9009.
    let out = cli(dir.path(), &["--", "heavy-cmd-no-such-program"], &[("HEAVY_CMD_HOME", &home("c"))]);
    assert_eq!(out.status.code(), Some(if cfg!(windows) { 9009 } else { 127 }));
    // A program heavy-cmd cannot start is its own to report: 127.
    let out = cli(dir.path(), &["--", "heavy-cmd-no-such-program", "x"], &[("HEAVY_CMD_HOME", &home("d"))]);
    assert_eq!(out.status.code(), Some(127));
    assert!(text(&out.stderr).starts_with("heavy-cmd: cannot run heavy-cmd-no-such-program: "));
}

#[test]
fn a_command_line_it_does_not_take_exits_64_a_setting_out_of_bounds_78_naming_why() {
    let dir = TempDir::new("heavy-cmd-cli");
    let run = |args: &[&str], env: &[(&str, &str)]| {
        let out = cli(dir.path(), args, env);
        (out.status.code().unwrap(), text(&out.stderr))
    };
    assert_eq!(run(&[], &[]).0, 64);
    let unknown = "heavy-cmd: unknown command \"npm\": a command to run goes after --, as in heavy-cmd -- npm test\n";
    assert_eq!(run(&["npm", "test"], &[]), (64, unknown.into()));
    assert_eq!(run(&["--wiat", "3", "--", "exit 0"], &[]), (64, "heavy-cmd: unknown option --wiat\n".into()));
    assert_eq!(run(&["--"], &[]), (64, "heavy-cmd: nothing to run after --\n".into()));
    assert_eq!(run(&["status", "--wait", "3"], &[]), (64, "heavy-cmd: status takes no --wait\n".into()));
    assert_eq!(run(&["logs", "--since", "yesterday"], &[]).0, 64);
    let cpu = "heavy-cmd: HEAVY_CMD_CPU must be an integer between 10 and 100, not \"5\"\n";
    assert_eq!(run(&["--", "exit 0"], &[("HEAVY_CMD_CPU", "5")]), (78, cpu.into()));
    let (code, stderr) = run(&["update"], &[]);
    assert_eq!(code, 1);
    assert!(stderr.contains("is a dev checkout: update it with git"), "{stderr}");
    assert!(logs(dir.path()).is_empty());
}

#[test]
fn one_that_cannot_start_within_wait_says_it_waits_then_exits_75_recorded_as_given_up() {
    let dir = TempDir::new("heavy-cmd-cli");
    let _holder = hold(dir.path()).unwrap();
    let out = cli(dir.path(), &["--wait", "3", "--label", "late", "--", "exit 0"], &[]);
    let stderr = text(&out.stderr);
    assert_eq!(out.status.code(), Some(75));
    assert!(stderr.starts_with("heavy-cmd: every heavy slot is busy: 1 running · cpu "), "{stderr}");
    assert!(stderr.contains(", last start ") && stderr.contains(" ago (holder-project: holder); waiting up to 3s\n"), "{stderr}");
    assert!(stderr.ends_with("heavy-cmd: no heavy command could start in 3s\n"), "{stderr}");
    let run = &logs(dir.path())[0];
    assert_eq!(
        (&run["label"], &run["exit"], &run["outcome"], &run["started_at"]),
        (&"late".into(), &75.into(), &"gave up".into(), &Value::Null)
    );
}

#[test]
fn inside_another_heavy_cmd_it_runs_at_once_recorded_as_inside_that_one() {
    let dir = TempDir::new("heavy-cmd-cli");
    let _holder = hold(dir.path()).unwrap();
    let out = cli(dir.path(), &["--wait", "0", "--agent", "task-1", "--", "exit 0"], &[("HEAVY_CMD_ID", "1")]);
    assert_eq!(out.status.code(), Some(0));
    let run = &logs(dir.path())[0];
    assert_eq!((&run["inside"], &run["agent"], &run["outcome"], &run["exit"]), (&1.into(), &"task-1".into(), &"ran".into(), &0.into()));
}

#[cfg(unix)]
#[test]
fn a_signal_while_it_waits_stops_the_wait_is_recorded_and_ends_heavy_cmd_as_it_would_the_command() {
    use std::io::{BufRead, BufReader};
    use std::os::unix::process::ExitStatusExt;
    let dir = TempDir::new("heavy-cmd-cli");
    let _holder = hold(dir.path()).unwrap();
    let mut waiter = heavy_cmd(dir.path()).args(["--label", "waiter", "--", "true"]).stderr(std::process::Stdio::piped()).spawn().unwrap();
    let mut stderr = BufReader::new(waiter.stderr.take().unwrap());
    let mut said = String::new();
    while !said.contains("every heavy slot is busy") {
        assert!(stderr.read_line(&mut said).unwrap() > 0, "ended before waiting: {said}");
    }
    std::process::Command::new("sh").args(["-c", &format!("kill -TERM {}", waiter.id())]).status().unwrap();
    assert_eq!(waiter.wait().unwrap().signal(), Some(15));
    let run = &logs(dir.path())[0];
    assert_eq!((&run["label"], &run["exit"], &run["outcome"]), (&"waiter".into(), &143.into(), &"stopped while waiting".into()));
}

#[cfg(windows)]
#[test]
fn a_bare_program_name_is_found_with_pathext_so_a_batch_file_runs() {
    let dir = TempDir::new("heavy-cmd-cli");
    std::fs::write(dir.join("tool.cmd"), "@exit /b 7\r\n").unwrap();
    let path = format!("{};{}", dir.path().display(), std::env::var("PATH").unwrap_or_default());
    assert_eq!(cli(dir.path(), &["--", "tool", "arg"], &[("PATH", &path)]).status.code(), Some(7));
}

#[test]
fn smoke_a_second_command_waits_out_the_gap_after_the_first_and_logs_shows_both() {
    let dir = TempDir::new("heavy-cmd-cli");
    let first = cli(dir.path(), &["--label", "first", "--agent", "smoke", "--", "exit 0"], &[]);
    assert_eq!((first.status.code(), text(&first.stderr)), (Some(0), String::new()));
    let before = Instant::now();
    let second = cli(dir.path(), &["--label", "second", "--agent", "smoke", "--", "exit 0"], &[]);
    assert!(before.elapsed().as_millis() >= 8_000);
    assert_eq!(second.status.code(), Some(0));
    assert!(text(&second.stderr).starts_with("heavy-cmd: every heavy slot is busy: 0 running"));
    let out = text(&cli(dir.path(), &["logs"], &[]).stdout);
    let lines: Vec<&str> = out.trim_end().lines().collect();
    assert_eq!(lines.len(), 2);
    let project = dir.name();
    let waited = lines[0].split("  waited ").nth(1).and_then(|rest| rest.split('s').next()).and_then(|s| s.parse::<u32>().ok());
    assert!(lines[0].contains(&format!("  {project}  second  exit 0  waited ")) && lines[0].ends_with("  ran 0s  smoke"), "{}", lines[0]);
    assert!(waited.is_some_and(|s| (9..20).contains(&s)), "{}", lines[0]);
    assert!(lines[1].ends_with(&format!("  {project}  first  exit 0  waited 0s  ran 0s  smoke")), "{}", lines[1]);
}
