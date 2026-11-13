use heavy_cmd::cpu::{CpuTimes, SAMPLES, measure, percent, usage_line};
use heavy_cmd::rusqlite::Connection;

fn samples() -> Connection {
    let db = Connection::open_in_memory().unwrap();
    db.execute_batch(SAMPLES).unwrap();
    db
}

fn at(ms: i64, pct: f64) -> CpuTimes {
    CpuTimes { busy: ms as f64 * pct / 100.0, total: ms as f64 }
}

fn kept(db: &Connection) -> Vec<i64> {
    let mut statement = db.prepare("SELECT at FROM samples ORDER BY at").unwrap();
    statement.query_map([], |row| row.get(0)).unwrap().map(Result::unwrap).collect()
}

#[test]
fn busy_over_total_between_two_readings_across_every_core() {
    assert_eq!(percent(CpuTimes { busy: 100.0, total: 1000.0 }, CpuTimes { busy: 400.0, total: 2000.0 }), Some(30.0));
    assert_eq!(percent(CpuTimes { busy: 0.0, total: 0.0 }, CpuTimes { busy: 2.0, total: 3.0 }), Some(66.7));
}

#[test]
fn counters_that_went_back_as_after_a_restart_give_no_reading() {
    assert_eq!(percent(CpuTimes { busy: 500.0, total: 1000.0 }, CpuTimes { busy: 10.0, total: 20.0 }), None);
    assert_eq!(percent(CpuTimes { busy: 0.0, total: 1000.0 }, CpuTimes { busy: 0.0, total: 1000.0 }), None);
}

#[test]
fn the_usage_is_over_the_last_10_seconds_of_samples_kept_in_heavy_db() {
    let db = samples();
    assert_eq!(measure(&db, 0, at(0, 0.0)).unwrap(), None);
    assert_eq!(measure(&db, 9_999, at(9_999, 0.0)).unwrap(), None);
    // From 0 to 10s the counters say 40% busy.
    assert_eq!(measure(&db, 10_000, CpuTimes { busy: 4_000.0, total: 10_000.0 }).unwrap(), Some(40.0));
}

#[test]
fn a_sample_is_kept_at_most_every_2_seconds_whoever_takes_it() {
    let db = samples();
    for ms in [0, 500, 1_000, 2_000, 3_000, 4_500] {
        measure(&db, ms, at(ms, 50.0)).unwrap();
    }
    assert_eq!(kept(&db), [0, 2_000, 4_500]);
}

#[test]
fn the_newest_sample_at_least_10_seconds_old_starts_the_window_one_older_than_15_is_stale() {
    let db = samples();
    measure(&db, 0, CpuTimes { busy: 0.0, total: 0.0 }).unwrap();
    measure(&db, 3_000, CpuTimes { busy: 3_000.0, total: 3_000.0 }).unwrap(); // 100% busy for the first 3 seconds
    assert_eq!(measure(&db, 13_000, CpuTimes { busy: 3_000.0, total: 13_000.0 }).unwrap(), Some(0.0));
    assert_eq!(measure(&db, 29_000, at(29_000, 50.0)).unwrap(), None);
}

#[test]
fn a_sample_ahead_of_the_clock_which_was_set_back_since_is_dropped() {
    let db = samples();
    measure(&db, 20_000, at(20_000, 50.0)).unwrap();
    measure(&db, 5_000, at(5_000, 50.0)).unwrap();
    assert_eq!(kept(&db), [5_000]);
}

#[test]
fn the_machines_counters_busy_never_more_than_the_total() {
    let machine = heavy_cmd::sys::cpu_times();
    assert!(machine.total > 0.0);
    assert!(machine.busy <= machine.total);
}

#[test]
fn shown_as_the_running_commands_and_the_usage_against_the_cpu_target() {
    assert_eq!(usage_line(2, Some(63.4), 80), "2 running · cpu 63%/80%");
    assert_eq!(usage_line(0, None, 80), "0 running · cpu ?/80%");
}
