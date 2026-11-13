//! heavy-cmd runs a heavy command (an install, a build, a test suite) once the machine has room: every
//! heavy-cmd on the machine joins one FIFO queue in `heavy.db`, and the oldest waiter starts once the last
//! start was 10 seconds ago and, while others run, the machine's CPU usage is under 60 %.

#[cfg(not(any(target_os = "linux", windows)))]
compile_error!("heavy-cmd runs on Linux and Windows");

pub mod cpu;
pub mod fail;
pub mod format;
pub mod queue;
pub mod run;
pub mod sys;

/// The crate the tests reach heavy.db through.
pub use rusqlite;
