use chrono::{DateTime, Datelike, Local, NaiveDate, NaiveDateTime, SecondsFormat, TimeZone, Utc};
use std::io::Write;

/// Now, in ms since the epoch.
pub fn now_ms() -> i64 {
    Utc::now().timestamp_millis()
}

/// `2026-11-12T10:00:00.000Z`: a time as the record keeps it.
pub fn iso(ms: i64) -> String {
    DateTime::<Utc>::from_timestamp_millis(ms).unwrap_or_default().to_rfc3339_opts(SecondsFormat::Millis, true)
}

/// The ms a time the record keeps names; None for anything else.
pub fn parse_iso(text: &str) -> Option<i64> {
    DateTime::parse_from_rfc3339(text).ok().map(|at| at.timestamp_millis())
}

/// An ISO time as a person may write it: `2026-11-12`, `2026-11-12T10:00`, with seconds, ms, `Z` or
/// `+02:00`. A date alone is UTC's midnight; a date and time with no zone is local, as JavaScript reads them.
pub fn parse_time(text: &str) -> Option<i64> {
    if let Some(ms) = parse_iso(text) {
        return Some(ms);
    }
    if text.len() == 10 {
        let date = NaiveDate::parse_from_str(text, "%Y-%m-%d").ok()?;
        return Some(date.and_hms_opt(0, 0, 0)?.and_utc().timestamp_millis());
    }
    let (naive, offset_ms) = if let Some(rest) = text.strip_suffix('Z') {
        (rest, Some(0))
    } else if text.len() > 16 && matches!(text.as_bytes()[text.len() - 6], b'+' | b'-') && text.as_bytes()[text.len() - 3] == b':' {
        let (rest, zone) = text.split_at(text.len() - 6);
        let hours: i64 = zone[1..3].parse().ok()?;
        let minutes: i64 = zone[4..].parse().ok()?;
        let sign = if zone.starts_with('-') { -1 } else { 1 };
        (rest, Some(sign * (hours * 60 + minutes) * 60_000))
    } else {
        (text, None)
    };
    let naive = naive.replacen(' ', "T", 1);
    let at = ["%Y-%m-%dT%H:%M:%S%.f", "%Y-%m-%dT%H:%M"].iter().find_map(|format| NaiveDateTime::parse_from_str(&naive, format).ok())?;
    match offset_ms {
        Some(offset) => Some(at.and_utc().timestamp_millis() - offset),
        None => Local.from_local_datetime(&at).earliest().map(|local| local.timestamp_millis()),
    }
}

fn local(ms: i64) -> DateTime<Local> {
    DateTime::<Utc>::from_timestamp_millis(ms).unwrap_or_default().with_timezone(&Local)
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

/// The local time of `at`, `14:02` (`14:02:11` with `seconds`), its date too on another day than `now`'s: `Jan 2 14:02`.
pub fn clock(at: i64, now: i64, seconds: bool) -> String {
    let date = local(at);
    let time = date.format(if seconds { "%H:%M:%S" } else { "%H:%M" }).to_string();
    if date.date_naive() == local(now).date_naive() {
        return time;
    }
    format!("{} {} {time}", date.format("%b"), date.day())
}

/// One line on stdout; a closed stdout is not heavy-cmd's failure.
pub fn say(text: &str) {
    let _ = writeln!(std::io::stdout(), "{text}");
}

/// One line on stderr, as `say`.
pub fn warn(text: &str) {
    let _ = writeln!(std::io::stderr(), "{text}");
}
