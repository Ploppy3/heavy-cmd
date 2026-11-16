//! `heavy-cmd` on a terminal: the queue live, the commands running, the next ones and the last 10 runs,
//! redrawn every second until a signal ends it.

use crate::fail::Result;
use crate::format::duration;
use crate::logs::{Run, clock, line, place};
use crate::queue::{CPU_TARGET, Heavy, HeavyStatus, pause};
use crate::run::summary;
use crate::sys;
use std::io::Write;
use std::path::Path;

/// How many of the last runs the view shows.
pub const HISTORY: u32 = 10;

/// How often a terminal's view is redrawn.
const REDRAW_MS: u64 = 1_000;

/// The view's lines: what the queue is at, the running commands, the next ones in the order they start,
/// and the last runs, the last ended first; each section `none` when it has nothing. A running command
/// shows when it started, a next one when it was queued, a run when it ended.
pub fn frame(status: &HeavyStatus, history: &[Run], target: u32, now: i64, home: Option<&Path>) -> Vec<String> {
    let mut lines = vec![format!("heavy-cmd v{} · {}", env!("CARGO_PKG_VERSION"), summary(status, target, now))];
    let row = |at, cwd: &str, command: &str, how: String| format!("  {}  {}  {command}  {how}", clock(at, now), place(cwd, home));
    let running = status.running.iter().map(|it| {
        let started = it.started_at.unwrap_or(it.queued_at);
        row(started, &it.cwd, &it.command, format!("waited {}  running {}", duration(started - it.queued_at), duration(now - started)))
    });
    section(&mut lines, "running", running);
    let next = status.waiting.iter().map(|it| row(it.queued_at, &it.cwd, &it.command, format!("waiting {}", duration(now - it.queued_at))));
    section(&mut lines, "next", next);
    section(&mut lines, "history", history.iter().map(|run| format!("  {}  {}", clock(run.ended_at, now), line(run, home))));
    lines
}

fn section(lines: &mut Vec<String>, title: &str, rows: impl Iterator<Item = String>) {
    lines.extend([String::new(), title.to_string()]);
    let before = lines.len();
    lines.extend(rows);
    if lines.len() == before {
        lines.push("  none".into());
    }
}

/// The frame as a terminal draws it over the last one, `cols` by `rows`: from the top left, each line
/// cleared then cut short of the last column so none wraps, as many lines as fit, the rest of the screen
/// cleared. A control character, such as a shell line's newline, shows as a space.
pub fn screen(lines: &[String], (cols, rows): (usize, usize)) -> String {
    let mut screen = String::from("\x1b[H");
    for (n, line) in lines.iter().take(rows).enumerate() {
        screen.push_str(if n == 0 { "\x1b[2K" } else { "\r\n\x1b[2K" });
        screen.extend(line.chars().take(cols.saturating_sub(1)).map(|c| if c.is_control() { ' ' } else { c }));
    }
    screen + "\x1b[J"
}

fn view(heavy: &Heavy, home: Option<&Path>) -> Result<Vec<String>> {
    Ok(frame(&heavy.status()?, &heavy.recent(HISTORY)?, CPU_TARGET, heavy.now(), home))
}

/// Shows the view of `heavy` on the terminal stdout is, `size` at first, on its alternate screen, redrawn
/// every second until a signal ends it with a Stopped failure.
pub fn watch(heavy: &Heavy, mut size: (usize, usize)) -> Result<()> {
    let home = std::env::home_dir();
    let _signals = sys::catch_signals();
    let mut out = std::io::stdout();
    let _alternate = Alternate::enter(&mut out)?;
    loop {
        size = sys::term_size().unwrap_or(size);
        out.write_all(screen(&view(heavy, home.as_deref())?, size).as_bytes())?;
        out.flush()?;
        pause(REDRAW_MS, Some(&sys::STOP))?;
    }
}

/// The terminal's alternate screen, the cursor hidden, until it drops: the screen before then comes back.
struct Alternate;

impl Alternate {
    fn enter(out: &mut impl Write) -> std::io::Result<Alternate> {
        out.write_all(b"\x1b[?1049h\x1b[?25l")?;
        out.flush()?;
        Ok(Alternate)
    }
}

impl Drop for Alternate {
    fn drop(&mut self) {
        let mut out = std::io::stdout();
        let _ = out.write_all(b"\x1b[?25h\x1b[?1049l");
        let _ = out.flush();
    }
}
