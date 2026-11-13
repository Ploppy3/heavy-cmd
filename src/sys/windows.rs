use super::STOP;
use crate::cpu::CpuTimes;
use crate::format::now_ms;
use std::ffi::OsString;
use std::io;
use std::os::windows::ffi::OsStringExt;
use std::os::windows::io::AsRawHandle;
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus};
use std::sync::atomic::{AtomicBool, Ordering};
use windows_sys::Win32::Foundation::{CloseHandle, ERROR_ACCESS_DENIED, FILETIME, GetLastError, HANDLE, STILL_ACTIVE};
use windows_sys::Win32::System::Console::{CTRL_BREAK_EVENT, CTRL_C_EVENT, CTRL_CLOSE_EVENT, SetConsoleCtrlHandler};
use windows_sys::Win32::System::JobObjects::{
    AssignProcessToJobObject, CreateJobObjectW, JOB_OBJECT_LIMIT_BREAKAWAY_OK, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
    JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JobObjectExtendedLimitInformation, SetInformationJobObject,
};
use windows_sys::Win32::System::SystemInformation::{ComputerNameDnsHostname, GetComputerNameExW, GetTickCount64};
use windows_sys::Win32::System::Threading::{
    CREATE_BREAKAWAY_FROM_JOB, CREATE_NEW_PROCESS_GROUP, DETACHED_PROCESS, GetExitCodeProcess, GetSystemTimes, OpenProcess,
    PROCESS_QUERY_LIMITED_INFORMATION,
};

/// What a shell line runs through, as the record and its messages name it.
pub const SHELL: &str = "cmd.exe";

/// The signal numbers the console's events stand for, as Windows' C runtime numbers them.
const SIGINT: i32 = 2;
const SIGBREAK: i32 = 21;
const SIGHUP: i32 = 1;

fn ms(time: FILETIME) -> f64 {
    ((u64::from(time.dwHighDateTime) << 32) | u64::from(time.dwLowDateTime)) as f64 / 10_000.0
}

/// The machine's CPU time since boot, in ms: busy is kernel time without idle, plus user time.
pub fn cpu_times() -> CpuTimes {
    let zero = FILETIME { dwLowDateTime: 0, dwHighDateTime: 0 };
    let (mut idle, mut kernel, mut user) = (zero, zero, zero);
    // SAFETY: three FILETIMEs to write into.
    if unsafe { GetSystemTimes(&mut idle, &mut kernel, &mut user) } == 0 {
        return CpuTimes::default();
    }
    let busy = ms(kernel) - ms(idle) + ms(user);
    CpuTimes { busy, total: ms(kernel) + ms(user) }
}

/// When the machine booted, in ms since the epoch.
pub fn booted_at() -> i64 {
    // SAFETY: no arguments.
    now_ms() - unsafe { GetTickCount64() } as i64
}

/// Whether `pid` names a live process: one this user may not open lives too.
pub fn pid_alive(pid: u32) -> bool {
    if pid == 0 {
        return false;
    }
    // SAFETY: the handle is checked, and closed once read.
    unsafe {
        let handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
        if handle.is_null() {
            return GetLastError() == ERROR_ACCESS_DENIED;
        }
        let mut code = 0u32;
        let read = GetExitCodeProcess(handle, &mut code);
        CloseHandle(handle);
        read != 0 && code == STILL_ACTIVE as u32
    }
}

pub fn hostname() -> String {
    let mut buffer = [0u16; 256];
    let mut len = buffer.len() as u32;
    // SAFETY: the buffer is as long as `len` says.
    if unsafe { GetComputerNameExW(ComputerNameDnsHostname, buffer.as_mut_ptr(), &mut len) } == 0 {
        return std::env::var("COMPUTERNAME").unwrap_or_default();
    }
    OsString::from_wide(&buffer[..len as usize]).to_string_lossy().into_owned()
}

/// A shell line, as `cmd.exe /d /s /c "<line>"` runs it: the line reaches cmd as it was written.
pub fn shell(line: &str) -> Command {
    let mut command = Command::new(std::env::var_os("ComSpec").unwrap_or_else(|| SHELL.into()));
    command.raw_arg(format!("/d /s /c \"{line}\""));
    command
}

/// `file` as Windows finds a bare name on PATH: with each of PATHEXT's extensions, so `npm` is `npm.cmd`.
fn resolve(file: &str) -> PathBuf {
    let path = Path::new(file);
    let exts = std::env::var("PATHEXT").unwrap_or_else(|_| ".COM;.EXE;.BAT;.CMD".into());
    let exts: Vec<&str> = exts.split(';').filter(|ext| !ext.is_empty()).collect();
    let bare = path.components().count() == 1 && !path.has_root();
    let known = path.extension().is_some_and(|ext| exts.iter().any(|known| known[1..].eq_ignore_ascii_case(&ext.to_string_lossy())));
    if !bare || known {
        return path.to_path_buf();
    }
    let dirs = std::env::var_os("PATH").unwrap_or_default();
    for dir in std::iter::once(PathBuf::from(".")).chain(std::env::split_paths(&dirs)) {
        for ext in &exts {
            let candidate = dir.join(format!("{file}{ext}"));
            if candidate.is_file() {
                return candidate;
            }
        }
    }
    path.to_path_buf()
}

/// A program and its arguments; a batch file runs through cmd, as Rust quotes its arguments for it.
pub fn program(file: &str, args: &[String]) -> Command {
    let mut command = Command::new(resolve(file));
    command.args(args);
    command
}

/// The exit of a command: its code, as Windows gives it.
pub fn exit_of(status: ExitStatus) -> i32 {
    status.code().unwrap_or(1)
}

/// Starts `command` with no console and out of this one's job, so that neither ending ends it. A job
/// that allows no breakaway keeps it.
pub fn detach(command: &mut Command) {
    command.creation_flags(DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP | CREATE_BREAKAWAY_FROM_JOB);
}

/// `detach` for a job that allows no breakaway.
pub fn detach_in_job(command: &mut Command) {
    command.creation_flags(DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP);
}

/// Whether a command runs: the console's events are then its own.
static CHILD: AtomicBool = AtomicBool::new(false);

unsafe extern "system" fn on_event(event: u32) -> i32 {
    if CHILD.load(Ordering::SeqCst) {
        // The console sends the event to the command too: heavy-cmd outlives it, to exit as it does.
        return 1;
    }
    STOP.signal(match event {
        CTRL_C_EVENT => SIGINT,
        CTRL_BREAK_EVENT => SIGBREAK,
        _ => SIGHUP,
    });
    if event >= CTRL_CLOSE_EVENT {
        // Windows ends the process once this returns: the wait records its stop first.
        std::thread::sleep(std::time::Duration::from_secs(4));
    }
    1
}

/// The console's events (Ctrl+C, Ctrl+Break, its closing) stop the wait, until it drops.
pub struct Signals(());

pub fn catch_signals() -> Signals {
    // SAFETY: the handler only touches atomics, and sleeps.
    unsafe { SetConsoleCtrlHandler(Some(on_event), 1) };
    Signals(())
}

impl Drop for Signals {
    fn drop(&mut self) {
        // SAFETY: removes the handler added above.
        unsafe { SetConsoleCtrlHandler(Some(on_event), 0) };
    }
}

/// While a command runs: it is in a job that ends with heavy-cmd, so whatever ends heavy-cmd ends it,
/// as a passed-on SIGTERM would. Once it is over the job lets go, and what it left running lives on.
pub struct Running(HANDLE);

fn limit(job: HANDLE, flags: u32) {
    // SAFETY: the information struct is zeroed but for its flags, and its size is given.
    unsafe {
        let mut info: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = std::mem::zeroed();
        info.BasicLimitInformation.LimitFlags = flags;
        let size = std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32;
        SetInformationJobObject(job, JobObjectExtendedLimitInformation, (&raw const info).cast(), size);
    }
}

pub fn running(child: &Child) -> Running {
    CHILD.store(true, Ordering::SeqCst);
    // SAFETY: a job with no name; the child's handle lives as long as `child`.
    let job = unsafe { CreateJobObjectW(std::ptr::null(), std::ptr::null()) };
    if !job.is_null() {
        limit(job, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE | JOB_OBJECT_LIMIT_BREAKAWAY_OK);
        // SAFETY: both handles are open.
        unsafe { AssignProcessToJobObject(job, child.as_raw_handle() as HANDLE) };
    }
    Running(job)
}

impl Drop for Running {
    fn drop(&mut self) {
        CHILD.store(false, Ordering::SeqCst);
        if !self.0.is_null() {
            limit(self.0, 0);
            // SAFETY: the job's only handle, closed once.
            unsafe { CloseHandle(self.0) };
        }
    }
}

/// Ends this process as a console event ended the wait: `128 + n`, as on Linux.
pub fn die_by(signal: i32) -> ! {
    std::process::exit(128 + signal)
}

/// Moves `path` out of the way: removed, or renamed aside while it still runs, as Windows allows.
fn discard(path: &Path) -> io::Result<()> {
    if !path.exists() || std::fs::remove_file(path).is_ok() {
        return Ok(());
    }
    let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
    std::fs::rename(path, path.with_file_name(format!(".discarded-{}-{nanos}.exe", std::process::id())))
}

/// Puts `new` in `bin`'s place, keeping the one before as `old`. A running program cannot be replaced
/// on Windows, but it can be renamed: `bin` becomes `old`, then `new` becomes `bin`; on a failure
/// between the two, `bin` comes back.
pub fn switch(bin: &Path, new: &Path, old: &Path) -> io::Result<()> {
    if let Some(dir) = bin.parent() {
        for entry in std::fs::read_dir(dir)?.flatten() {
            if entry.file_name().to_string_lossy().starts_with(".discarded-") {
                let _ = std::fs::remove_file(entry.path());
            }
        }
    }
    discard(old)?;
    if bin.exists() {
        std::fs::rename(bin, old)?;
    }
    std::fs::rename(new, bin).inspect_err(|_| {
        let _ = std::fs::rename(old, bin);
    })
}
