use std::io::Write;
use std::time::{SystemTime, UNIX_EPOCH};

/// Now, in ms since the epoch.
pub fn now_ms() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |since| since.as_millis() as i64)
}

/// `42s`, `3m 4s`, `2h 5m`.
pub fn duration(ms: i64) -> String {
    let s = ((ms as f64) / 1000.0).round().max(0.0) as i64;
    if s < 60 {
        return format!("{s}s");
    }
    let m = s / 60;
    if m < 60 {
        return if s % 60 != 0 { format!("{m}m {}s", s % 60) } else { format!("{m}m") };
    }
    if m % 60 != 0 { format!("{}h {}m", m / 60, m % 60) } else { format!("{}h", m / 60) }
}

/// One line on stdout; a closed stdout is not heavy-cmd's failure.
pub fn say(text: &str) {
    let _ = writeln!(std::io::stdout(), "{text}");
}

/// One line on stderr, as `say`.
pub fn warn(text: &str) {
    let _ = writeln!(std::io::stderr(), "{text}");
}
