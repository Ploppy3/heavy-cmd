//! `heavy-cmd` as a process: its command lines, the view on a terminal (Linux), exits, nesting, a signal
//! while waiting, a test build's refused update, and the smoke test: two commands in a temp home, the
//! second waiting out the 10 s gap, and `logs` showing both.

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
    let mut options = ClaimOptions { wait_sec: 0, ..ClaimOptions::default() };
    heavy.claim(&mut options, std::process::id(), "held by the suite", &dir.display().to_string()).unwrap();
    heavy
}

#[test]
fn off_a_terminal_anything_but_a_command_logs_or_update_prints_the_usage_and_exits_64() {
    let dir = TempDir::new("heavy-cmd-cli");
    // Bare off a terminal too: a heavy-cmd checks the release it updates to by its usage's first line.
    for args in [&[][..], &["--"], &["npm", "test"], &["--version"], &["logs", "--json"], &["update", "--check"]] {
        let out = cli(dir.path(), args, &[]);
        assert_eq!(out.status.code(), Some(64), "{args:?}");
        let usage = text(&out.stderr);
        assert!(usage.starts_with(&format!("heavy-cmd v{}: ", env!("CARGO_PKG_VERSION"))), "{usage}");
        for line in ["\n  heavy-cmd\n", "\n  heavy-cmd -- <command>...\n", "\n  heavy-cmd logs\n", "\n  heavy-cmd update\n"] {
            assert!(usage.contains(line), "{usage}");
        }
    }
    let out = cli(dir.path(), &["logs"], &[]);
    assert_eq!((out.status.code(), text(&out.stdout)), (Some(0), "no heavy command recorded\n".into()));
    assert!(!state(dir.path()).exists());
}

#[cfg(unix)]
#[test]
fn bare_on_a_terminal_it_draws_the_view_until_a_signal_then_gives_the_screen_back() {
    use heavy_cmd::logs::place;
    use std::io::Read;
    use std::os::fd::{FromRawFd, OwnedFd};
    use std::os::unix::process::ExitStatusExt;
    let dir = TempDir::new("heavy-cmd-cli");
    let _holder = hold(dir.path());
    let (mut master, mut slave) = (0, 0);
    let size = libc::winsize { ws_row: 24, ws_col: 200, ws_xpixel: 0, ws_ypixel: 0 };
    // SAFETY: two fds to write into, and the terminal's size; each fd is owned once opened.
    let (master, slave) = unsafe {
        assert_eq!(libc::openpty(&mut master, &mut slave, std::ptr::null_mut(), std::ptr::null(), &size), 0);
        (OwnedFd::from_raw_fd(master), OwnedFd::from_raw_fd(slave))
    };
    let mut view = heavy_cmd(dir.path()).stdout(slave).spawn().unwrap();
    let (send, drawn) = std::sync::mpsc::channel();
    let mut master = std::fs::File::from(master);
    std::thread::spawn(move || {
        let mut chunk = [0; 4096];
        // The terminal reads EIO once the view ends.
        while let Ok(n @ 1..) = master.read(&mut chunk) {
            let _ = send.send(chunk[..n].to_vec());
        }
    });
    let mut screen = Vec::new();
    while !text(&screen).contains("\x1b[J") {
        screen.extend(drawn.recv_timeout(Duration::from_secs(10)).expect("no view drawn"));
    }
    let first = text(&screen);
    assert!(first.starts_with("\x1b[?1049h\x1b[?25l\x1b[H\x1b[2Kheavy-cmd v"), "{first:?}");
    let place = place(&dir.path().display().to_string(), std::env::home_dir().as_deref());
    assert!(
        first.contains("running\r\n\x1b[2K  ") && first.contains(&format!("  {place}  held by the suite  waited 0s  running ")),
        "{first:?}"
    );
    assert!(first.contains("next\r\n\x1b[2K  none\r\n"), "{first:?}");
    std::process::Command::new("sh").args(["-c", &format!("kill -INT {}", view.id())]).status().unwrap();
    assert_eq!(view.wait().unwrap().signal(), Some(2));
    screen.extend(drawn.iter().flatten());
    assert!(text(&screen).ends_with("\x1b[?25h\x1b[?1049l"), "{:?}", text(&screen));
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
fn a_build_that_is_not_a_release_s_never_updates() {
    let dir = TempDir::new("heavy-cmd-cli");
    let out = cli(dir.path(), &["update"], &[]);
    assert_eq!(out.status.code(), Some(1));
    assert_eq!(text(&out.stderr), "heavy-cmd: this build does not update itself: only a release's binary does\n");
    assert_eq!(cli(dir.path(), &["--", "exit 0"], &[]).status.code(), Some(0));
    assert!(state(dir.path()).join("heavy.db").exists() && !state(dir.path()).join("update.log").exists());
}

#[test]
fn smoke_a_second_command_waits_out_the_gap_after_the_first_and_logs_shows_both() {
    let dir = TempDir::new("heavy-cmd-cli");
    let first = cli(dir.path(), &["--", "echo first"], &[]);
    assert_eq!((first.status.code(), text(&first.stderr)), (Some(0), String::new()));
    let before = Instant::now();
    let second = cli(dir.path(), &["--", "echo second"], &[]);
    assert!(before.elapsed().as_millis() >= 8_000);
    assert_eq!(second.status.code(), Some(0));
    assert!(text(&second.stderr).starts_with("heavy-cmd: every heavy slot is busy: 0 running"));
    let out = text(&cli(dir.path(), &["logs"], &[]).stdout);
    let lines: Vec<&str> = out.trim_end().lines().collect();
    assert_eq!(lines.len(), 2, "{out}");
    let waited = lines[0].split("  waited ").nth(1).and_then(|rest| rest.split('s').next()).and_then(|s| s.parse::<u32>().ok());
    assert!(lines[0].contains("  echo second  exit 0  waited ") && lines[0].ends_with("  ran 0s"), "{}", lines[0]);
    assert!(waited.is_some_and(|s| (8..20).contains(&s)), "{}", lines[0]);
    assert!(lines[1].ends_with("  echo first  exit 0  waited 0s  ran 0s"), "{}", lines[1]);
}
