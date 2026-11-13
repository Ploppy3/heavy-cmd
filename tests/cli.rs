//! `heavy-cmd` as a process: its one command line, exits, nesting, a signal while waiting, and the smoke
//! test: two commands in a temp home, the second waiting out the 10 s gap.

mod common;

use common::{TempDir, heavy_cmd, line, state};
use heavy_cmd::queue::{ClaimOptions, Heavy, HeavyPorts};
use std::path::Path;
use std::process::Output;
use std::time::{Duration, Instant};

fn cli(dir: &Path, args: &[&str], env: &[(&str, &str)]) -> Output {
    heavy_cmd(dir).args(args).envs(env.iter().copied()).output().unwrap()
}

fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).replace("\r\n", "\n")
}

/// This process holds a start in `dir`'s queue until the Heavy drops.
fn hold(dir: &Path) -> Heavy {
    let heavy = Heavy::open(&state(dir), HeavyPorts::machine()).unwrap();
    heavy.claim(&mut ClaimOptions { wait_sec: 0, ..ClaimOptions::default() }, std::process::id()).unwrap();
    heavy
}

#[test]
fn anything_but_dashdash_and_a_command_prints_the_usage_and_exits_64() {
    let dir = TempDir::new("heavy-cmd-cli");
    for args in [&[][..], &["--"], &["npm", "test"], &["--version"]] {
        let out = cli(dir.path(), args, &[]);
        assert_eq!(out.status.code(), Some(64), "{args:?}");
        let usage = text(&out.stderr);
        assert!(usage.starts_with(&format!("heavy-cmd v{}: ", env!("CARGO_PKG_VERSION"))), "{usage}");
        assert!(usage.contains("\n  heavy-cmd -- <command>...\n"), "{usage}");
    }
    assert!(!state(dir.path()).exists());
}

#[test]
fn runs_the_command_in_the_foreground_its_output_its_exit_one_argument_is_a_shell_line() {
    // Each in a home of its own: a second start in one home waits out the gap.
    let home = |name: &str| TempDir::new(&format!("heavy-cmd-cli-{name}"));
    let shell = line("echo out; echo err >&2; exit 3", "echo out& echo err 1>&2& exit 3");
    let out = cli(home("a").path(), &["--", &shell], &[]);
    assert_eq!((out.status.code(), text(&out.stdout).trim_end(), text(&out.stderr).trim_end()), (Some(3), "out", "err"));
    let id = line(r#"test -n "$HEAVY_CMD_ID" && echo "$HEAVY_CMD_ID""#, "echo %HEAVY_CMD_ID%");
    let out = cli(home("b").path(), &["--", &id], &[]);
    assert_eq!((out.status.code(), text(&out.stdout).trim_end()), (Some(0), "1"));
    // A shell line's unknown program is the shell's to report: sh exits 127, cmd.exe 9009.
    let out = cli(home("c").path(), &["--", "heavy-cmd-no-such-program"], &[]);
    assert_eq!(out.status.code(), Some(if cfg!(windows) { 9009 } else { 127 }));
    // A program heavy-cmd cannot start is its own to report: 127.
    let out = cli(home("d").path(), &["--", "heavy-cmd-no-such-program", "x"], &[]);
    assert_eq!(out.status.code(), Some(127));
    assert!(text(&out.stderr).starts_with("heavy-cmd: cannot run heavy-cmd-no-such-program: "));
}

#[test]
fn inside_another_heavy_cmd_it_runs_at_once() {
    let dir = TempDir::new("heavy-cmd-cli");
    let _holder = hold(dir.path());
    let mut nested = heavy_cmd(dir.path()).args(["--", "exit 5"]).env("HEAVY_CMD_ID", "1").spawn().unwrap();
    let before = Instant::now();
    while nested.try_wait().unwrap().is_none() {
        if before.elapsed() > Duration::from_secs(5) {
            let _ = nested.kill();
            panic!("it waited");
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    assert_eq!(nested.wait().unwrap().code(), Some(5));
}

#[cfg(unix)]
#[test]
fn a_signal_while_it_waits_stops_the_wait_and_ends_heavy_cmd_as_it_would_the_command() {
    use std::io::{BufRead, BufReader};
    use std::os::unix::process::ExitStatusExt;
    let dir = TempDir::new("heavy-cmd-cli");
    let _holder = hold(dir.path());
    let mut waiter = heavy_cmd(dir.path()).args(["--", "true"]).stderr(std::process::Stdio::piped()).spawn().unwrap();
    let mut stderr = BufReader::new(waiter.stderr.take().unwrap());
    let mut said = String::new();
    while !said.contains("every heavy slot is busy") {
        assert!(stderr.read_line(&mut said).unwrap() > 0, "ended before waiting: {said}");
    }
    assert!(said.ends_with("; waiting up to 900s\n"), "{said}");
    std::process::Command::new("sh").args(["-c", &format!("kill -TERM {}", waiter.id())]).status().unwrap();
    assert_eq!(waiter.wait().unwrap().signal(), Some(15));
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
fn smoke_a_second_command_waits_out_the_gap_after_the_first() {
    let dir = TempDir::new("heavy-cmd-cli");
    let first = cli(dir.path(), &["--", "exit 0"], &[]);
    assert_eq!((first.status.code(), text(&first.stderr)), (Some(0), String::new()));
    let before = Instant::now();
    let second = cli(dir.path(), &["--", "exit 0"], &[]);
    assert!(before.elapsed().as_millis() >= 8_000);
    assert_eq!(second.status.code(), Some(0));
    assert!(text(&second.stderr).starts_with("heavy-cmd: every heavy slot is busy: 0 running"));
}
