use std::fmt;

/// A command line heavy-cmd does not take: sysexits' EX_USAGE.
pub const EX_USAGE: i32 = 64;
/// The command could not start in time: sysexits' EX_TEMPFAIL, so it never reads as the command's own exit.
pub const NO_SLOT_EXIT: i32 = 75;

/// Why heavy-cmd stops short of its command's own exit; `code` is what it then exits with.
#[derive(Debug)]
pub enum Fail {
    /// No start came within the wait.
    Busy(String),
    /// A signal, by its number, stopped the wait.
    Stopped(i32),
    /// Any other failure of heavy-cmd's own, such as a heavy.db it cannot open.
    Other(String),
}

impl Fail {
    pub fn code(&self) -> i32 {
        match self {
            Fail::Busy(_) => NO_SLOT_EXIT,
            Fail::Stopped(signal) => 128 + signal,
            Fail::Other(_) => 1,
        }
    }
}

impl fmt::Display for Fail {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Fail::Busy(text) | Fail::Other(text) => f.write_str(text),
            Fail::Stopped(signal) => write!(f, "stopped by signal {signal}"),
        }
    }
}

impl std::error::Error for Fail {}

impl From<rusqlite::Error> for Fail {
    fn from(error: rusqlite::Error) -> Self {
        Fail::Other(error.to_string())
    }
}

impl From<std::io::Error> for Fail {
    fn from(error: std::io::Error) -> Self {
        Fail::Other(error.to_string())
    }
}

pub type Result<T> = std::result::Result<T, Fail>;
