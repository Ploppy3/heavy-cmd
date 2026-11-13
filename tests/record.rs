mod common;

use common::{World, line, options, pids, tick};
use heavy_cmd::chrono::{Local, TimeZone};
use heavy_cmd::cli::Flags;
use heavy_cmd::commands::{cmd_logs, run_line};
use heavy_cmd::config::Env;
use heavy_cmd::fail::{Fail, other};
use heavy_cmd::format::iso;
use heavy_cmd::queue::{ClaimOptions, Ran, hostname, read_runs};
use heavy_cmd::run::{heavily, run_heavy};
use heavy_cmd::runs::{HeavyRun, RunFilter};
use heavy_cmd::rusqlite::Connection;
use heavy_cmd::serde_json;
use heavy_cmd::sys::Stop;

/// The runs the world's record keeps, the last ended first.
fn runs(world: &World, limit: u64) -> Vec<HeavyRun> {
    read_runs(world.dir.path(), &RunFilter { limit, ..RunFilter::default() }).unwrap()
}

fn argv(words: &[&str]) -> Vec<String> {
    words.iter().map(|word| word.to_string()).collect()
}

#[test]
fn a_command_is_recorded_once_it_is_over_every_field_its_exit_its_wait_and_its_run() {
    let world = World::new();
    let h = world.heavy();
    h.release(world.claim(&h, 101, |_| {}).unwrap().who.id).unwrap();
    world.advance(10_000);
    let mut suite = ClaimOptions { label: "suite".into(), wait_sec: 60, cwd: Some("/work/api".into()), ..options() };
    assert_eq!(run_heavy(&h, &argv(&["exit 3"]), &mut suite).unwrap(), 3);
    let at = iso(world.now());
    let expected = HeavyRun {
        id: 1,
        project: "api".into(),
        label: "suite".into(),
        command: Some(argv(&["exit 3"])),
        cwd: "/work/api".into(),
        host: hostname().into(),
        pid: i64::from(std::process::id()),
        agent: Some("task-73".into()),
        heavy_id: Some(2),
        inside: None,
        queued_at: at.clone(),
        started_at: Some(at.clone()),
        ended_at: at,
        exit: Some(3),
        outcome: "ran".into(),
    };
    assert_eq!(runs(&world, 50), [expected]);
    // Several arguments are kept as argv.
    world.advance(10_000);
    let several = if cfg!(windows) { argv(&["cmd", "/c", "exit 0"]) } else { argv(&["sh", "-c", "exit 0"]) };
    assert_eq!(run_heavy(&h, &several, &mut options()).unwrap(), 0);
    let last = &runs(&world, 1)[0];
    assert_eq!((last.command.as_ref(), last.exit), (Some(&several), Some(0)));
    assert_eq!(last.cwd, std::env::current_dir().unwrap().display().to_string());
}

#[test]
fn a_wait_that_gives_up_is_recorded_with_75_never_started() {
    let world = World::new();
    let h = world.heavy();
    world.claim(&h, 101, |_| {}).unwrap();
    world.advance(5_000);
    assert!(matches!(run_heavy(&h, &argv(&["exit 0"]), &mut options()), Err(Fail::Busy(_))));
    let run = &runs(&world, 50)[0];
    assert_eq!((run.label.as_str(), run.queued_at.clone(), run.started_at.clone()), ("npm test", iso(world.now()), None));
    assert_eq!((run.exit, run.outcome.as_str(), run.pid), (Some(75), "gave up", i64::from(std::process::id())));
}

#[test]
fn a_wait_stopped_by_a_signal_is_recorded_with_128_plus_it_another_stop_with_no_exit() {
    let world = World::new();
    let h = world.heavy();
    world.set_cpu(100.0);
    world.claim(&h, 101, |_| {}).unwrap();
    for signal in [Some(15), None] {
        let stop: &'static Stop = Box::leak(Box::new(Stop::new()));
        std::thread::scope(|s| {
            let waiting = s.spawn(|| {
                let mut waiter = ClaimOptions { wait_sec: 60, stop: Some(stop), ..options() };
                heavily(&world.heavy(), &mut waiter, |_| Ok("ran"), |_| 0)
            });
            tick();
            world.advance(3_000);
            signal.map_or_else(|| stop.abort(), |signal| stop.signal(signal));
            assert!(matches!(waiting.join().unwrap(), Err(Fail::Stopped(_))));
        });
    }
    let how: Vec<_> = runs(&world, 50).into_iter().map(|run| (run.outcome, run.exit, run.started_at)).collect();
    assert_eq!(how, [("stopped while waiting".into(), None, None), ("stopped while waiting".into(), Some(143), None)]);
    assert_eq!(pids(&h.status().unwrap()), (vec![101], vec![]));
}

#[test]
fn a_command_inside_another_is_recorded_as_run_inside_that_ones_id_started_at_once() {
    let world = World::new();
    let h = world.heavy();
    let mut inner = ClaimOptions { label: "inner".into(), inside: Some("7".into()), ..options() };
    assert_eq!(run_heavy(&h, &argv(&["exit 0"]), &mut inner).unwrap(), 0);
    let run = &runs(&world, 50)[0];
    let at = iso(world.now());
    assert_eq!((run.label.as_str(), run.inside, run.heavy_id, run.exit), ("inner", Some(7), None, Some(0)));
    assert_eq!((run.queued_at.clone(), run.started_at.clone()), (at.clone(), Some(at)));
}

#[test]
fn work_that_is_not_a_command_is_recorded_with_exit_0_or_1_when_it_fails_and_no_command() {
    let world = World::new();
    let h = world.heavy();
    let warm = || ClaimOptions { label: "warm cache".into(), ..options() };
    assert_eq!(heavily(&h, &mut warm(), |_| Ok("built"), |_| 0).unwrap(), "built");
    world.advance(10_000);
    let failed = heavily(&h, &mut warm(), |_| Err::<&str, _>(other("no image")), |_| 0);
    assert_eq!(failed.unwrap_err().to_string(), "no image");
    let how: Vec<_> = runs(&world, 50).into_iter().map(|run| (run.label, run.command, run.exit)).collect();
    assert_eq!(how, [("warm cache".into(), None, Some(1)), ("warm cache".into(), None, Some(0))]);
}

#[test]
fn no_agent_is_recorded_as_none_and_the_command_sees_the_agent_it_ran_for() {
    let world = World::new();
    let h = world.heavy();
    let out = world.dir.join("agent");
    let shell =
        line(&format!(r#"printf %s "$HEAVY_CMD_AGENT" > '{}'"#, out.display()), &format!("echo %HEAVY_CMD_AGENT%> \"{}\"", out.display()));
    assert_eq!(run_heavy(&h, &[shell], &mut ClaimOptions { agent: Some("task-9".into()), ..options() }).unwrap(), 0);
    world.advance(10_000);
    run_heavy(&h, &argv(&["exit 0"]), &mut ClaimOptions { agent: None, label: "anonymous".into(), ..options() }).unwrap();
    let agents: Vec<_> = runs(&world, 50).into_iter().map(|run| (run.label, run.agent)).collect();
    assert_eq!(agents, [("anonymous".into(), None), ("npm test".into(), Some("task-9".into()))]);
    assert_eq!(std::fs::read_to_string(out).unwrap().trim(), "task-9");
}

#[test]
fn a_new_run_forgets_those_that_ended_over_30_days_before_it() {
    let world = World::new();
    let h = world.heavy();
    let ran = |label: &str| {
        let at = iso(world.now());
        let ran = Ran { queued_at: at.clone(), started_at: Some(at), exit: Some(0), outcome: "ran", ..Ran::default() };
        h.record(&ClaimOptions { label: label.into(), ..options() }, ran);
    };
    ran("old");
    world.advance(30 * 86_400_000);
    ran("a month");
    world.advance(1);
    ran("new");
    assert_eq!(runs(&world, 50).into_iter().map(|run| run.label).collect::<Vec<_>>(), ["new", "a month"]);
}

#[test]
fn a_record_that_cannot_be_written_leaves_the_exit_as_it_is_and_says_one_line_on_stderr() {
    let world = World::new();
    let h = world.heavy();
    let db = Connection::open(world.dir.join("heavy.db")).unwrap();
    db.execute_batch("CREATE TRIGGER full BEFORE INSERT ON runs BEGIN SELECT RAISE(FAIL, 'database or disk is full'); END").unwrap();
    drop(db);
    assert_eq!(run_heavy(&h, &argv(&["exit 5"]), &mut options()).unwrap(), 5);
    assert_eq!(world.warned(), ["heavy-cmd: the heavy command's run was not recorded: database or disk is full"]);
    assert!(h.status().unwrap().running.is_empty());
}

/// Today's noon, local: every run below ended today, so the log dates none.
fn noon() -> i64 {
    let today = Local::now().date_naive().and_hms_opt(12, 0, 0).unwrap();
    Local.from_local_datetime(&today).earliest().unwrap().timestamp_millis()
}

fn time(ms: i64) -> String {
    Local.timestamp_millis_opt(ms).unwrap().format("%H:%M:%S").to_string()
}

#[test]
fn logs_lists_the_machines_runs_the_last_ended_first_one_line_each_json_gives_every_field() {
    let world = World::new();
    let env = Env::of(&[("HEAVY_CMD_HOME", &world.dir.path().to_string_lossy())]);
    let now = noon();
    assert_eq!(cmd_logs(&Flags::default(), &env, now).unwrap(), "no heavy command recorded");
    let h = world.heavy();
    world.set_clock(now - 2 * 3_600_000);
    run_heavy(&h, &argv(&["exit 0"]), &mut options()).unwrap();
    world.set_clock(now - 600_000);
    h.release(world.claim(&h, 101, |o| o.project = "web".into()).unwrap().who.id).unwrap();
    world.advance(1_000);
    std::thread::scope(|s| {
        let running = s.spawn(|| {
            let mut check = ClaimOptions { project: "web".into(), label: "npm run check".into(), wait_sec: 60, ..options() };
            run_heavy(&world.heavy(), &argv(&["exit 0"]), &mut check)
        });
        tick();
        world.advance(11_000);
        assert_eq!(running.join().unwrap().unwrap(), 0);
    });
    world.advance(3_000);
    let mut build = ClaimOptions { project: "web".into(), label: "npm run build".into(), ..options() };
    assert!(matches!(run_heavy(&h, &argv(&["exit 0"]), &mut build), Err(Fail::Busy(_))));
    let text = cmd_logs(&Flags::default(), &env, now).unwrap();
    let lines: Vec<&str> = text.lines().collect();
    assert_eq!(lines[0], format!("{}  web  npm run build  gave up after 0s (75)  task-73", time(world.now())));
    assert_eq!(lines[1], format!("{}  web  npm run check  exit 0  waited 11s  ran 0s  task-73", time(world.now() - 3_000)));
    assert!(lines[2].ends_with("  api  npm test  exit 0  waited 0s  ran 0s  task-73"), "{}", lines[2]);
    let json: Vec<serde_json::Value> = serde_json::from_str(&cmd_logs(&Flags::of(&[("json", "")]), &env, now).unwrap()).unwrap();
    assert_eq!(json.iter().map(|run| run["label"].as_str().unwrap()).collect::<Vec<_>>(), ["npm run build", "npm run check", "npm test"]);
    let keys: Vec<&str> = json[0].as_object().unwrap().keys().map(String::as_str).collect();
    let fields = ["id", "project", "label", "command", "cwd", "host", "pid", "agent", "heavy_id", "inside"];
    assert_eq!(keys, [&fields[..], &["queued_at", "started_at", "ended_at", "exit", "outcome"]].concat());
}

#[test]
fn logs_project_limit_and_since_an_iso_time_or_a_duration_filter_it() {
    let world = World::new();
    let env = Env::of(&[("HEAVY_CMD_HOME", &world.dir.path().to_string_lossy())]);
    let now = noon();
    let h = world.heavy();
    let ran = |project: &str, label: &str, ago: i64| {
        world.set_clock(now - ago);
        let at = iso(world.now());
        let options = ClaimOptions { project: project.into(), label: label.into(), ..ClaimOptions::default() };
        h.record(&options, Ran { queued_at: at.clone(), started_at: Some(at), exit: Some(0), outcome: "ran", ..Ran::default() });
    };
    ran("api", "two hours", 2 * 3_600_000);
    ran("web", "half an hour", 1_800_000);
    ran("api", "a minute", 60_000);
    let labels = |pairs: &[(&str, &str)]| -> Vec<String> {
        let flags = Flags::of(&[pairs, &[("json", "")]].concat());
        let json: Vec<serde_json::Value> = serde_json::from_str(&cmd_logs(&flags, &env, now).unwrap()).unwrap();
        json.iter().map(|run| run["label"].as_str().unwrap().to_string()).collect()
    };
    assert_eq!(labels(&[("project", "api")]), ["a minute", "two hours"]);
    assert_eq!(labels(&[("limit", "2")]), ["a minute", "half an hour"]);
    assert_eq!(labels(&[("since", "1h")]), ["a minute", "half an hour"]);
    assert_eq!(labels(&[("since", "45m"), ("project", "web")]), ["half an hour"]);
    assert_eq!(labels(&[("since", &iso(now - 1_000_000))]), ["a minute"]);
    let refused = |pairs: &[(&str, &str)]| cmd_logs(&Flags::of(pairs), &env, now).unwrap_err().to_string();
    assert_eq!(refused(&[("since", "yesterday")]), "--since takes an ISO time or a duration such as 1h, not \"yesterday\"");
    assert_eq!(refused(&[("limit", "-1")]), "--limit must be a non-negative integer");
}

#[test]
fn a_runs_line_on_another_day_carries_its_date_a_stopped_wait_with_no_exit_says_none() {
    let run = HeavyRun {
        label: "npm ci".into(),
        project: "api".into(),
        inside: Some(4),
        queued_at: "2026-01-02T10:00:00.000Z".into(),
        ended_at: "2026-01-02T10:00:42.000Z".into(),
        outcome: "stopped while waiting".into(),
        ..HeavyRun::default()
    };
    let ended = Local.timestamp_millis_opt(heavy_cmd::format::parse_iso(&run.ended_at).unwrap()).unwrap();
    let date = ended.format("%b %-d %H:%M:%S").to_string();
    let now = heavy_cmd::format::parse_iso("2026-03-01T12:00:00Z").unwrap();
    assert_eq!(run_line(&run, now), format!("{date}  api  npm ci  stopped while waiting after 42s  inside heavy 4"));
}
