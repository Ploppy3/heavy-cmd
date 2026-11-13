//! What the suites share: temp folders, the binary under test and a fake machine for the queue.
#![allow(dead_code)]

use heavy_cmd::cpu::CpuTimes;
use heavy_cmd::fail::Result;
use heavy_cmd::queue::{ClaimOptions, Heavy, HeavyPorts, HeavyStatus};
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicI64, AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// A folder of its own, gone once dropped.
pub struct TempDir(PathBuf);

impl TempDir {
    pub fn new(prefix: &str) -> TempDir {
        static COUNT: AtomicU32 = AtomicU32::new(0);
        let n = COUNT.fetch_add(1, Ordering::SeqCst);
        let dir = std::env::temp_dir().join(format!("{prefix}-{}-{n}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        TempDir(dir)
    }

    pub fn path(&self) -> &Path {
        &self.0
    }

    pub fn join(&self, path: &str) -> PathBuf {
        self.0.join(path)
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// The heavy-cmd binary under test.
pub fn bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_heavy-cmd"))
}

/// The folder `heavy_cmd(dir)` keeps heavy.db in.
pub fn state(dir: &Path) -> PathBuf {
    dir.join("state").join("heavy-cmd")
}

/// `bin` as the suites run it: on a temp state of `dir`'s, never the machine's, and outside any heavy-cmd
/// the suite itself runs in.
pub fn heavy_cmd(dir: &Path) -> Command {
    let mut command = Command::new(bin());
    let name = if cfg!(windows) { "LOCALAPPDATA" } else { "XDG_STATE_HOME" };
    command.current_dir(dir).env(name, dir.join("state")).env_remove("HEAVY_CMD_ID");
    command
}

/// The shell line for this system: `unix` through sh, `windows` through cmd.exe.
pub fn line(unix: &str, windows: &str) -> String {
    if cfg!(windows) { windows.to_string() } else { unix.to_string() }
}

pub fn tick() {
    std::thread::sleep(Duration::from_millis(20));
}

/// How the queue suites wait: wait 0, a CPU target of 80, a poll of 1 ms.
pub fn options() -> ClaimOptions {
    ClaimOptions { wait_sec: 0, cpu: 80, poll_ms: 1, ..ClaimOptions::default() }
}

/// The machine's CPU usage, in %, from the last sample on, and the counters it made so far.
struct Cpu {
    percent: f64,
    at: i64,
    times: CpuTimes,
}

/// A machine the queue runs on whose clock, CPU, boot and live pids only the test moves; each `heavy()`
/// is one process's connection to the same heavy.db.
pub struct World {
    pub dir: TempDir,
    alive: Arc<Mutex<HashSet<u32>>>,
    booted: Arc<AtomicI64>,
    clock: Arc<AtomicI64>,
    cpu: Arc<Mutex<Cpu>>,
}

impl World {
    pub fn new() -> World {
        let clock = 1_000_000;
        World {
            dir: TempDir::new("heavy-cmd-queue"),
            alive: Arc::new(Mutex::new(HashSet::from([std::process::id()]))),
            booted: Arc::new(AtomicI64::new(0)),
            clock: Arc::new(AtomicI64::new(clock)),
            cpu: Arc::new(Mutex::new(Cpu { percent: 0.0, at: clock, times: CpuTimes::default() })),
        }
    }

    pub fn heavy(&self) -> Heavy {
        let (alive, booted, clock, now) = (self.alive.clone(), self.booted.clone(), self.clock.clone(), self.clock.clone());
        let cpu = self.cpu.clone();
        let ports = HeavyPorts {
            pid_alive: Box::new(move |pid| alive.lock().unwrap().contains(&pid)),
            booted_at: Box::new(move || booted.load(Ordering::SeqCst)),
            now: Box::new(move || now.load(Ordering::SeqCst)),
            sample: Box::new(move || {
                let now = clock.load(Ordering::SeqCst);
                let mut cpu = cpu.lock().unwrap();
                let elapsed = (now - cpu.at) as f64;
                cpu.times.busy += elapsed * cpu.percent / 100.0;
                cpu.times.total += elapsed;
                cpu.at = now;
                cpu.times
            }),
        };
        Heavy::open(self.dir.path(), ports).unwrap()
    }

    pub fn now(&self) -> i64 {
        self.clock.load(Ordering::SeqCst)
    }

    pub fn advance(&self, ms: i64) {
        self.clock.fetch_add(ms, Ordering::SeqCst);
    }

    pub fn set_cpu(&self, percent: f64) {
        self.cpu.lock().unwrap().percent = percent;
    }

    pub fn set_booted(&self, at: i64) {
        self.booted.store(at, Ordering::SeqCst);
    }

    pub fn kill(&self, pid: u32) {
        self.alive.lock().unwrap().remove(&pid);
    }

    /// `pid` queues a command on `heavy`, waiting as `edit` changes `options()`, and returns its id once it starts.
    pub fn claim(&self, heavy: &Heavy, pid: u32, edit: impl FnOnce(&mut ClaimOptions)) -> Result<i64> {
        self.alive.lock().unwrap().insert(pid);
        let mut options = options();
        edit(&mut options);
        heavy.claim(&mut options, pid)
    }
}

/// The pids running, and those waiting, in order.
pub fn pids(status: &HeavyStatus) -> (Vec<u32>, Vec<u32>) {
    (status.running.clone(), status.waiting.clone())
}
