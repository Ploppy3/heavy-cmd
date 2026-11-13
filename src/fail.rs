use std::fmt;

/// A command line heavy-cmd does not take: sysexits' EX_USAGE.
pub const EX_USAGE: i32 = 64;
/// A setting out of its bounds: sysexits' EX_CONFIG.
pub const EX_CONFIG: i32 = 78;
/// The command could not start in time: sysexits' EX_TEMPFAIL, so it never reads as the command's own exit.
pub const NO_SLOT_EXIT: i32 = 75;

/// Why heavy-cmd stops short of its command's own exit; `code` is what it then exits with.
#[derive(Debug)]
pub enum Fail {
    Usage(String),
    Config(String),
    /// No start came within the wait.
    Busy(String),
    /// The wait was stopped: by that signal's number, or by something else.
    Stopped(Option<i32>),
    /// Any other failure of heavy-cmd's own, such as an update.
    Other(String),
}

impl Fail {
    pub fn code(&self) -> i32 {
        match self {
            Fail::Usage(_) => EX_USAGE,
            Fail::Config(_) => EX_CONFIG,
            Fail::Busy(_) => NO_SLOT_EXIT,
            Fail::Stopped(Some(signal)) => 128 + signal,
            Fail::Stopped(None) | Fail::Other(_) => 1,
        }
    }
}

impl fmt::Display for Fail {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Fail::Usage(text) | Fail::Config(text) | Fail::Busy(text) | Fail::Other(text) => f.write_str(text),
            Fail::Stopped(Some(signal)) => write!(f, "stopped by signal {signal}"),
            Fail::Stopped(None) => f.write_str("stopped"),
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

/// A failure of heavy-cmd's own, exiting 1.
pub fn other(text: impl Into<String>) -> Fail {
    Fail::Other(text.into())
}
