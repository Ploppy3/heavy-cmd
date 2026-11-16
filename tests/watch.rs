//! The view bare `heavy-cmd` shows: its lines, and how a terminal draws them over the last.

use heavy_cmd::logs::{RAN, Run, clock};
use heavy_cmd::queue::{HeavyStatus, Queued};
use heavy_cmd::watch::{frame, screen};
use std::path::MAIN_SEPARATOR as SEP;

const NOW: i64 = 1_800_000_000_000;

fn queued(command: &str, queued_at: i64, started_at: Option<i64>) -> Queued {
    Queued { id: 1, pid: 101, command: command.into(), cwd: std::env::temp_dir().join("api").display().to_string(), queued_at, started_at }
}

#[test]
fn the_view_says_what_the_queue_is_at_then_the_running_commands_the_next_and_the_last_runs() {
    let home = std::env::temp_dir();
    let status = HeavyStatus {
        running: vec![queued("npm run check", NOW - 196_000, Some(NOW - 184_000))],
        waiting: vec![queued("npm ci", NOW - 62_000, None), queued("cargo build", NOW - 5_000, None)],
        cpu: Some(63.4),
        last_start: Some(NOW - 12_000),
    };
    let ran = Run {
        command: "npm test".into(),
        cwd: "/elsewhere".into(),
        queued_at: NOW - 300_000,
        started_at: Some(NOW - 288_000),
        ended_at: NOW - 104_000,
        exit: Some(0),
        outcome: RAN,
    };
    let at = |ms: i64| clock(NOW - ms, NOW);
    let version = env!("CARGO_PKG_VERSION");
    assert_eq!(
        frame(&status, &[ran], 60, NOW, Some(&home)),
        [
            format!("heavy-cmd v{version} · 1 running · cpu 63%/60%, last start 12s ago"),
            String::new(),
            "running".into(),
            format!("  {}  ~{SEP}api  npm run check  waited 12s  running 3m 4s", at(184_000)),
            String::new(),
            "next".into(),
            format!("  {}  ~{SEP}api  npm ci  waiting 1m 2s", at(62_000)),
            format!("  {}  ~{SEP}api  cargo build  waiting 5s", at(5_000)),
            String::new(),
            "history".into(),
            format!("  {}  /elsewhere  npm test  exit 0  waited 12s  ran 3m 4s", at(104_000)),
        ]
    );
}

#[test]
fn a_section_with_nothing_says_none() {
    let status = HeavyStatus { running: vec![], waiting: vec![], cpu: None, last_start: None };
    let version = env!("CARGO_PKG_VERSION");
    assert_eq!(
        frame(&status, &[], 60, NOW, None),
        [
            &format!("heavy-cmd v{version} · 0 running · cpu ?/60%, no start yet"),
            "",
            "running",
            "  none",
            "",
            "next",
            "  none",
            "",
            "history",
            "  none",
        ]
    );
}

#[test]
fn a_terminal_draws_it_from_the_top_left_each_line_cleared_and_cut_short_of_the_last_column() {
    let lines = ["header".to_string(), "  npm ci\nnpm test".into(), "cut".into()];
    assert_eq!(screen(&lines, (80, 24)), "\x1b[H\x1b[2Kheader\r\n\x1b[2K  npm ci npm test\r\n\x1b[2Kcut\x1b[J");
    // As many lines as it has rows, none as wide as it is: no line wraps and the screen never scrolls.
    assert_eq!(screen(&lines, (6, 2)), "\x1b[H\x1b[2Kheade\r\n\x1b[2K  npm\x1b[J");
}
