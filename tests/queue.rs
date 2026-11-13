mod common;

use common::{World, line, options, pids, tick};
use heavy_cmd::fail::Fail;
use heavy_cmd::queue::HeavyStatus;
use heavy_cmd::run::{run_heavy, waiting_line};
use std::sync::{Arc, Mutex};

fn busy<T: std::fmt::Debug>(result: Result<T, Fail>) {
    assert!(matches!(result, Err(Fail::Busy(_))), "not busy: {result:?}");
}

#[test]
fn with_none_running_and_no_start_yet_a_command_starts_at_once() {
    let world = World::new();
    let h = world.heavy();
    world.claim(&h, 101, |_| {}).unwrap();
    assert_eq!(pids(&h.status().unwrap()), (vec![101], vec![]));
}

#[test]
fn under_the_cpu_target_a_second_command_starts_10_seconds_after_the_first_while_it_runs() {
    let world = World::new();
    let h = world.heavy();
    world.set_cpu(30.0);
    world.claim(&h, 101, |_| {}).unwrap();
    world.advance(10_000);
    world.claim(&h, 102, |_| {}).unwrap();
    assert_eq!(pids(&h.status().unwrap()).0, [101, 102]);
}

#[test]
fn over_the_cpu_target_with_one_running_a_command_waits_then_gives_up_with_nothing_left() {
    let world = World::new();
    let h = world.heavy();
    world.set_cpu(95.0);
    world.claim(&h, 101, |_| {}).unwrap();
    world.advance(10_000);
    let refused = world.claim(&h, 102, |_| {}).unwrap_err();
    // heavy-cmd exits with it: sysexits' EX_TEMPFAIL, never the command's own code.
    assert!(matches!(refused, Fail::Busy(_)));
    assert_eq!(refused.code(), 75);
    assert_eq!(pids(&h.status().unwrap()), (vec![101], vec![]));
    world.set_cpu(50.0);
    world.advance(10_000);
    world.claim(&h, 103, |_| {}).unwrap();
    assert_eq!(pids(&h.status().unwrap()).0, [101, 103]);
}

#[test]
fn a_cpu_usage_not_measured_yet_counts_as_over_the_target_while_one_runs() {
    let world = World::new();
    let h = world.heavy();
    world.claim(&h, 101, |_| {}).unwrap();
    world.advance(16_000); // the only sample is stale: no reading over the last 10 seconds
    busy(world.claim(&h, 102, |_| {}));
}

#[test]
fn ten_seconds_pass_between_two_starts_whatever_the_cpu_and_whether_the_first_still_runs() {
    let world = World::new();
    let h = world.heavy();
    let first = world.claim(&h, 101, |_| {}).unwrap();
    world.advance(9_999);
    busy(world.claim(&h, 102, |_| {}));
    h.release(first).unwrap();
    busy(world.claim(&h, 102, |_| {}));
    world.advance(1);
    world.claim(&h, 102, |_| {}).unwrap();
}

#[test]
fn a_clock_set_back_starts_the_gap_again_from_then_without_skipping_it() {
    let world = World::new();
    let h = world.heavy();
    h.release(world.claim(&h, 101, |_| {}).unwrap()).unwrap();
    world.advance(-60_000);
    busy(world.claim(&h, 102, |_| {}));
    world.advance(9_999);
    busy(world.claim(&h, 102, |_| {}));
    world.advance(1);
    world.claim(&h, 102, |_| {}).unwrap();
}

#[test]
fn with_none_running_the_oldest_waiter_starts_once_the_gap_has_passed_even_at_100_cpu() {
    let world = World::new();
    let h = world.heavy();
    world.set_cpu(100.0);
    h.release(world.claim(&h, 101, |_| {}).unwrap()).unwrap();
    std::thread::scope(|s| {
        let waiting = s.spawn(|| world.claim(&world.heavy(), 102, |o| o.wait_sec = 60));
        tick();
        assert_eq!(pids(&h.status().unwrap()).1, [102]);
        world.advance(10_000);
        waiting.join().unwrap().unwrap();
        assert_eq!(pids(&h.status().unwrap()).0, [102]);
    });
}

#[test]
fn two_waiters_in_two_processes_never_start_within_the_same_gap_and_the_oldest_starts_first() {
    let world = World::new();
    let a = world.heavy();
    world.claim(&a, 101, |_| {}).unwrap();
    std::thread::scope(|s| {
        let older = s.spawn(|| world.claim(&world.heavy(), 102, |o| o.wait_sec = 60));
        tick();
        let younger = s.spawn(|| world.claim(&world.heavy(), 103, |o| o.wait_sec = 60));
        tick();
        assert_eq!(pids(&a.status().unwrap()).1, [102, 103]);
        world.advance(10_000);
        older.join().unwrap().unwrap();
        tick();
        assert_eq!(pids(&a.status().unwrap()), (vec![101, 102], vec![103]));
        world.advance(10_000);
        younger.join().unwrap().unwrap();
        assert_eq!(pids(&a.status().unwrap()).0, [101, 102, 103]);
    });
}

#[test]
fn a_waiter_whose_process_died_no_longer_holds_the_queue() {
    let world = World::new();
    let h = world.heavy();
    h.release(world.claim(&h, 101, |_| {}).unwrap()).unwrap();
    std::thread::scope(|s| {
        let dead = s.spawn(|| world.claim(&world.heavy(), 102, |o| o.wait_sec = 15));
        tick();
        let next = s.spawn(|| world.claim(&world.heavy(), 103, |o| o.wait_sec = 60));
        world.kill(102);
        world.advance(10_000);
        next.join().unwrap().unwrap();
        assert_eq!(pids(&h.status().unwrap()).0, [103]);
        world.advance(10_000);
        busy(dead.join().unwrap());
    });
}

#[test]
fn a_holder_whose_process_is_gone_or_which_predates_the_last_boot_no_longer_counts() {
    let world = World::new();
    let h = world.heavy();
    world.set_cpu(100.0);
    world.claim(&h, 101, |_| {}).unwrap();
    world.kill(101);
    world.advance(10_000);
    world.claim(&h, 102, |_| {}).unwrap();
    assert_eq!(pids(&h.status().unwrap()).0, [102]);
    world.set_booted(world.now() + 1);
    world.advance(10_000);
    world.claim(&h, 103, |_| {}).unwrap();
    assert_eq!(pids(&h.status().unwrap()).0, [103]);
}

#[test]
fn a_wait_is_said_once_with_the_running_commands_and_the_cpu_usage() {
    let world = World::new();
    let h = world.heavy();
    world.set_cpu(90.0);
    world.claim(&h, 101, |_| {}).unwrap();
    let told: Arc<Mutex<Vec<HeavyStatus>>> = Arc::default();
    std::thread::scope(|s| {
        let into = told.clone();
        let waiting = s.spawn(|| {
            world.claim(&world.heavy(), 102, move |o| {
                o.wait_sec = 60;
                o.on_wait = Some(Box::new(move |status| into.lock().unwrap().push(status.clone())));
            })
        });
        tick();
        world.advance(10_000);
        tick();
        world.set_cpu(10.0);
        world.advance(10_000);
        waiting.join().unwrap().unwrap();
    });
    let told = told.lock().unwrap();
    assert_eq!(told.len(), 1);
    assert_eq!(pids(&told[0]), (vec![101], vec![102]));
    assert_eq!(told[0].last_start, Some(1_000_000));
}

#[test]
fn the_waiting_line_says_how_many_run_the_cpu_against_its_target_and_the_last_start() {
    let world = World::new();
    let h = world.heavy();
    world.set_cpu(85.0);
    world.claim(&h, 101, |_| {}).unwrap();
    world.advance(12_000);
    let line = waiting_line(&h.status().unwrap(), 80, 900, world.now());
    assert_eq!(line, "heavy-cmd: every heavy slot is busy: 1 running · cpu 85%/80%, last start 12s ago; waiting up to 900s");
    let none = HeavyStatus { running: vec![], waiting: vec![], cpu: None, last_start: None };
    assert_eq!(
        waiting_line(&none, 60, 900, 0),
        "heavy-cmd: every heavy slot is busy: 0 running · cpu ?/60%, no start yet; waiting up to 900s"
    );
}

#[test]
fn run_heavy_one_argument_is_a_shell_line_its_exit_is_passed_on_and_it_stops_counting() {
    let world = World::new();
    let h = world.heavy();
    let shell = line(r#"test -n "$HEAVY_CMD_ID" && exit 3"#, "if defined HEAVY_CMD_ID exit 3");
    assert_eq!(run_heavy(&h, &[shell], &mut options()).unwrap(), 3);
    assert!(h.status().unwrap().running.is_empty());
}

#[test]
fn run_heavy_several_arguments_run_the_program_directly() {
    let world = World::new();
    let argv: Vec<String> = if cfg!(windows) { ["cmd", "/c", "exit 4"] } else { ["sh", "-c", "exit 4"] }.map(String::from).to_vec();
    assert_eq!(run_heavy(&world.heavy(), &argv, &mut options()).unwrap(), 4);
}

#[test]
fn run_heavy_a_program_that_cannot_start_exits_127_and_stops_counting() {
    let world = World::new();
    let h = world.heavy();
    let argv = ["heavy-cmd-no-such-program".to_string(), "x".to_string()];
    assert_eq!(run_heavy(&h, &argv, &mut options()).unwrap(), 127);
    assert!(h.status().unwrap().running.is_empty());
}

#[test]
fn run_heavy_a_command_that_could_not_start_in_time_is_an_error_and_never_runs() {
    let world = World::new();
    let h = world.heavy();
    world.claim(&h, 101, |_| {}).unwrap();
    let ran = world.dir.join("ran");
    let shell = line(&format!("touch '{}'", ran.display()), &format!("type nul > \"{}\"", ran.display()));
    busy(run_heavy(&h, &[shell], &mut options()));
    assert_eq!(pids(&h.status().unwrap()), (vec![101], vec![]));
    assert!(!ran.exists());
}
