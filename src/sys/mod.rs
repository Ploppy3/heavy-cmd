//! What differs between Linux and Windows: CPU counters, boot time, process liveness, the shell, signals
//! (console events on Windows), detaching a process and keeping a command to its heavy-cmd.

use std::sync::atomic::{AtomicI32, Ordering};

#[cfg(unix)]
mod unix;
#[cfg(unix)]
pub use unix::*;

#[cfg(windows)]
mod windows;
#[cfg(windows)]
pub use windows::*;

/// What stops a wait: a signal, by its number, or another reason. Set from a signal handler, so only atomics.
#[derive(Debug, Default)]
pub struct Stop(AtomicI32);

impl Stop {
    pub const fn new() -> Stop {
        Stop(AtomicI32::new(0))
    }

    /// Stops by `signal`, as its number.
    pub fn signal(&self, signal: i32) {
        let _ = self.0.compare_exchange(0, signal, Ordering::SeqCst, Ordering::SeqCst);
    }

    /// Stops for a reason that is no signal.
    pub fn abort(&self) {
        let _ = self.0.compare_exchange(0, -1, Ordering::SeqCst, Ordering::SeqCst);
    }

    /// None while it goes on; once stopped, the signal that stopped it, if one did.
    pub fn reason(&self) -> Option<Option<i32>> {
        match self.0.load(Ordering::SeqCst) {
            0 => None,
            -1 => Some(None),
            signal => Some(Some(signal)),
        }
    }
}

/// What the process's signals stop: the wait of the heavy-cmd this process runs.
pub static STOP: Stop = Stop::new();
