//! What differs between Linux and Windows: CPU counters, boot time, process liveness, the shell, signals
//! (console events on Windows) and keeping a command to its heavy-cmd.

use std::sync::atomic::{AtomicI32, Ordering};

#[cfg(unix)]
mod unix;
#[cfg(unix)]
pub use unix::*;

#[cfg(windows)]
mod windows;
#[cfg(windows)]
pub use windows::*;

/// What stops a wait: a signal, by its number. Set from a signal handler, so only atomics.
#[derive(Debug, Default)]
pub struct Stop(AtomicI32);

impl Stop {
    pub const fn new() -> Stop {
        Stop(AtomicI32::new(0))
    }

    /// Stops by `signal`, as its number; the first one counts.
    pub fn signal(&self, signal: i32) {
        let _ = self.0.compare_exchange(0, signal, Ordering::SeqCst, Ordering::SeqCst);
    }

    /// The signal that stopped it; None while it goes on.
    pub fn signalled(&self) -> Option<i32> {
        Some(self.0.load(Ordering::SeqCst)).filter(|signal| *signal != 0)
    }
}

/// What the process's signals stop: the wait of the heavy-cmd this process runs.
pub static STOP: Stop = Stop::new();
