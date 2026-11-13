//! The record: every run once it is over, kept 30 days, and the lines `heavy-cmd logs` shows.

mod common;

use common::{World, options};
use heavy_cmd::fail::Fail;
use heavy_cmd::logs::{GAVE_UP, RAN, Run, STOPPED, clock, line, recent, record};
use heavy_cmd::queue::ClaimOptions;
use heavy_cmd::run::run_heavy;
use heavy_cmd::rusqlite::Connection;
use heavy_cmd::sys::Stop;
use std::path::Path;

fn argv(words: &[&str]) -> Vec<String> {
    words.iter().map(|word| word.to_string()).collect()
}

fn cwd() -> String {
    std::env::current_dir().unwrap().display().to_string()
}

#[test]
fn a_run_is_recorded_once_over_its_command_cwd_times_exit_and_outcome() {
    let world = World::new();
    let h = world.heavy();
    assert!(recent(world.dir.path(), 50).unwrap().is_empty());
    assert_eq!(run_heavy(&h, &argv(&["exit 3"]), &mut options(), false).unwrap(), 3);
    let at = world.now();
    let ran = Run { command: "exit 3".into(), cwd: cwd(), queued_at: at, started_at: Some(at), ended_at: at, exit: Some(3), outcome: RAN };
    assert_eq!(recent(world.dir.path(), 50).unwrap(), [ran]);
    // Several arguments are kept joined; the last ended comes first.
    world.advance(10_000);
    let several = if cfg!(windows) { argv(&["cmd", "/c", "exit 0"]) } else { argv(&["sh", "-c", "exit 0"]) };
    run_heavy(&h, &several, &mut options(), false).unwrap();
    let runs = recent(world.dir.path(), 50).unwrap();
    assert_eq!((runs.len(), runs[0].command.as_str(), runs[0].exit), (2, several.join(" ").as_str(), Some(0)));
    assert_eq!(recent(world.dir.path(), 1).unwrap().len(), 1);
}

#[test]
fn one_that_never_started_is_recorded_as_given_up_or_stopped_with_heavy_cmds_exit() {
    static STOP: Stop = Stop::new();
    let world = World::new();
    let h = world.heavy();
    world.claim(&h, 101, |_| {}).unwrap();
    assert!(matches!(run_heavy(&h, &argv(&["exit 0"]), &mut options(), false), Err(Fail::Busy(_))));
    STOP.signal(15);
    let mut stopped = ClaimOptions { stop: Some(&STOP), ..options() };
    assert!(matches!(run_heavy(&h, &argv(&["exit 0"]), &mut stopped, false), Err(Fail::Stopped(15))));
    let runs = recent(world.dir.path(), 50).unwrap();
    let how: Vec<_> = runs.iter().map(|run| (run.outcome, run.exit, run.started_at)).collect();
    assert_eq!(how, [(STOPPED, Some(143), None), (GAVE_UP, Some(75), None)]);
}

#[test]
fn inside_another_heavy_command_it_runs_at_once_and_is_recorded() {
    let world = World::new();
    let h = world.heavy();
    world.set_cpu(100.0);
    world.claim(&h, 101, |_| {}).unwrap();
    assert_eq!(run_heavy(&h, &argv(&["exit 0"]), &mut options(), true).unwrap(), 0);
    assert_eq!(h.status().unwrap().running, [101]);
    let runs = recent(world.dir.path(), 50).unwrap();
    assert_eq!((runs[0].outcome, runs[0].exit, runs[0].started_at), (RAN, Some(0), Some(world.now())));
}

#[test]
fn a_write_forgets_the_runs_over_30_days_old() {
    let world = World::new();
    drop(world.heavy());
    let db = Connection::open(world.dir.join("heavy.db")).unwrap();
    let day = 86_400_000;
    let run =
        |ended_at| Run { command: "c".into(), cwd: "/".into(), queued_at: 0, started_at: None, ended_at, exit: None, outcome: GAVE_UP };
    record(&db, &run(0), 0).unwrap();
    record(&db, &run(day), day).unwrap();
    record(&db, &run(30 * day + 1), 30 * day + 1).unwrap();
    let kept: Vec<i64> = recent(world.dir.path(), 50).unwrap().iter().map(|run| run.ended_at).collect();
    assert_eq!(kept, [30 * day + 1, day]);
}

#[test]
fn a_line_says_where_what_and_how_it_went_the_home_as_tilde() {
    let home = std::env::temp_dir();
    let cwd = home.join("api").display().to_string();
    let ran =
        Run { command: "npm test".into(), cwd, queued_at: 0, started_at: Some(12_000), ended_at: 196_000, exit: Some(0), outcome: RAN };
    let sep = std::path::MAIN_SEPARATOR;
    assert_eq!(line(&ran, Some(&home)), format!("~{sep}api  npm test  exit 0  waited 12s  ran 3m 4s"));
    let gave_up = Run { started_at: None, ended_at: 900_000, exit: Some(75), outcome: GAVE_UP, ..ran.clone() };
    assert_eq!(line(&gave_up, Some(Path::new("/elsewhere"))), format!("{}  npm test  gave up after 15m (75)", ran.cwd));
    assert_eq!(line(&Run { cwd: home.display().to_string(), ..ran }, Some(&home)), "~  npm test  exit 0  waited 12s  ran 3m 4s");
}

#[test]
fn when_it_ended_is_a_local_time_its_date_too_on_another_day() {
    let now = heavy_cmd::format::now_ms();
    assert_eq!(clock(now, now).len(), "14:02:11".len());
    assert!(clock(now - 3 * 86_400_000, now).len() > "14:02:11".len());
}
