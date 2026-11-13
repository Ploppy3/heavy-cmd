use super::STOP;
use crate::cpu::CpuTimes;
use crate::format::now_ms;
use std::io;
use std::os::unix::process::{CommandExt, ExitStatusExt};
use std::path::Path;
use std::process::{Child, Command, ExitStatus};
use std::sync::atomic::{AtomicI32, Ordering};

/// What a shell line runs through, as the record and its messages name it.
pub const SHELL: &str = "sh";

/// The machine's CPU time since boot, from `/proc/stat`'s first line: busy is user, nice, system and
/// irq; idle is idle; in ms, as Node's `os.cpus()` counts them, so both read one heavy.db alike.
pub fn cpu_times() -> CpuTimes {
    let text = std::fs::read_to_string("/proc/stat").unwrap_or_default();
    let line = text.lines().find(|line| line.starts_with("cpu ")).unwrap_or("");
    let ticks: Vec<f64> = line.split_whitespace().skip(1).map(|value| value.parse().unwrap_or(0.0)).collect();
    let at = |index: usize| ticks.get(index).copied().unwrap_or(0.0);
    // SAFETY: sysconf only reads a system setting.
    let per_second = unsafe { libc::sysconf(libc::_SC_CLK_TCK) }.max(1) as f64;
    let ms = 1000.0 / per_second;
    let busy = (at(0) + at(1) + at(2) + at(5)) * ms;
    CpuTimes { busy, total: busy + at(3) * ms }
}

/// When the machine booted, in ms since the epoch.
pub fn booted_at() -> i64 {
    let uptime = std::fs::read_to_string("/proc/uptime").unwrap_or_default();
    let seconds: f64 = uptime.split_whitespace().next().and_then(|value| value.parse().ok()).unwrap_or(0.0);
    now_ms() - (seconds * 1000.0) as i64
}

/// Whether `pid` names a live process: one this user may not signal lives too.
pub fn pid_alive(pid: u32) -> bool {
    let Ok(pid) = i32::try_from(pid) else { return false };
    if pid <= 0 {
        return false;
    }
    // SAFETY: signal 0 checks the process exists, and sends nothing.
    (unsafe { libc::kill(pid, 0) } == 0) || io::Error::last_os_error().raw_os_error() == Some(libc::EPERM)
}

pub fn hostname() -> String {
    let mut buffer = [0u8; 256];
    // SAFETY: the buffer is as long as its length says, and gethostname writes at most that.
    if unsafe { libc::gethostname(buffer.as_mut_ptr().cast(), buffer.len()) } != 0 {
        return String::new();
    }
    let end = buffer.iter().position(|b| *b == 0).unwrap_or(buffer.len());
    String::from_utf8_lossy(&buffer[..end]).into_owned()
}

/// A shell line, as `sh -c` runs it.
pub fn shell(line: &str) -> Command {
    let mut command = Command::new(SHELL);
    command.arg("-c").arg(line);
    command
}

/// A program and its arguments, found on PATH.
pub fn program(file: &str, args: &[String]) -> Command {
    let mut command = Command::new(file);
    command.args(args);
    command
}

/// The exit of a command as a shell reports it: its code, `128 + n` when signal `n` ended it.
pub fn exit_of(status: ExitStatus) -> i32 {
    status.code().or_else(|| status.signal().map(|signal| 128 + signal)).unwrap_or(128)
}

/// Starts `command` in a session of its own, so the terminal's signals and its closing never reach it.
pub fn detach(command: &mut Command) {
    // SAFETY: setsid is async-signal-safe, as a pre_exec hook must be.
    unsafe {
        command.pre_exec(|| {
            libc::setsid();
            Ok(())
        });
    }
}

/// The signals heavy-cmd acts on: while it waits they stop the wait, once its command runs they are the command's.
const SIGNALS: [i32; 3] = [libc::SIGINT, libc::SIGTERM, libc::SIGHUP];

/// The running command's pid, 0 while there is none.
static CHILD: AtomicI32 = AtomicI32::new(0);

extern "C" fn on_signal(signal: libc::c_int) {
    let child = CHILD.load(Ordering::SeqCst);
    if child == 0 {
        STOP.signal(signal);
    } else if signal != libc::SIGINT {
        // SAFETY: kill is async-signal-safe. The terminal's SIGINT reaches the command through its process group.
        unsafe { libc::kill(child, signal) };
    }
}

/// The signals' handlers until it drops, when the ones before come back. A signal the process was
/// started ignoring, as under nohup, stays ignored: its command then ignores it too.
pub struct Signals(Vec<(i32, libc::sigaction)>);

pub fn catch_signals() -> Signals {
    let mut before = Vec::new();
    for signal in SIGNALS {
        // SAFETY: the actions are zeroed then filled in, and the handler only touches atomics and kill.
        unsafe {
            let mut old: libc::sigaction = std::mem::zeroed();
            libc::sigaction(signal, std::ptr::null(), &mut old);
            if old.sa_sigaction == libc::SIG_IGN {
                continue;
            }
            let mut action: libc::sigaction = std::mem::zeroed();
            action.sa_sigaction = on_signal as extern "C" fn(libc::c_int) as usize;
            action.sa_flags = libc::SA_RESTART;
            libc::sigemptyset(&mut action.sa_mask);
            libc::sigaction(signal, &action, std::ptr::null_mut());
            before.push((signal, old));
        }
    }
    Signals(before)
}

impl Drop for Signals {
    fn drop(&mut self) {
        for (signal, old) in &self.0 {
            // SAFETY: puts back the action read before.
            unsafe { libc::sigaction(*signal, old, std::ptr::null_mut()) };
        }
    }
}

/// While a command runs, the signals are passed on to it; until it drops.
pub struct Running(());

pub fn running(child: &Child) -> Running {
    CHILD.store(child.id() as i32, Ordering::SeqCst);
    Running(())
}

impl Drop for Running {
    fn drop(&mut self) {
        CHILD.store(0, Ordering::SeqCst);
    }
}

/// Ends this process by `signal`, as it would have ended the command: a shell sees `128 + n`.
pub fn die_by(signal: i32) -> ! {
    // SAFETY: the default action of these signals ends the process.
    unsafe {
        libc::signal(signal, libc::SIG_DFL);
        libc::raise(signal);
    }
    std::process::exit(128 + signal)
}

/// Puts `new` in `bin`'s place with one rename, keeping the one before as `old`.
pub fn switch(bin: &Path, new: &Path, old: &Path) -> io::Result<()> {
    let _ = std::fs::remove_file(old);
    if bin.exists() {
        std::fs::hard_link(bin, old).or_else(|_| std::fs::copy(bin, old).map(|_| ()))?;
    }
    std::fs::rename(new, bin)
}
