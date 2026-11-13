//! heavy-cmd runs a heavy command (an install, a build, a test suite) once the machine has room: every
//! heavy-cmd on the machine joins one FIFO queue in `heavy.db`, and the oldest waiter starts once the last
//! start was 10 seconds ago and, while others run, the machine's CPU usage is under its target.

#[cfg(not(any(target_os = "linux", windows)))]
compile_error!("heavy-cmd runs on Linux and Windows");

pub mod cli;
pub mod commands;
pub mod config;
pub mod cpu;
pub mod fail;
pub mod format;
pub mod install;
pub mod project;
pub mod queue;
pub mod run;
pub mod runs;
pub mod sys;
pub mod update;

/// The crates the tests reach heavy.db and the clock through.
pub use chrono;
pub use rusqlite;
pub use serde_json;
