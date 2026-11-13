mod common;

use common::TempDir;
use heavy_cmd::config::{Config, DEFAULTS, Env, config_file, load_config, state_dir};
use heavy_cmd::fail::Fail;
use std::path::{Path, PathBuf};

/// An env whose home is `home`, and nothing else.
fn env_in(home: &TempDir) -> Env {
    let home = home.path().to_string_lossy();
    Env::of(&[("HOME", &home), ("USERPROFILE", &home)])
}

/// Writes the config file `env` reads.
fn config(env: &Env, text: &str) -> PathBuf {
    let file = config_file(env);
    std::fs::create_dir_all(file.parent().unwrap()).unwrap();
    std::fs::write(&file, text).unwrap();
    file
}

fn refused(env: &Env, wait: Option<&str>) -> String {
    match load_config(env, wait) {
        Err(Fail::Config(why)) => why,
        other => panic!("not refused: {other:?}"),
    }
}

#[cfg(unix)]
#[test]
fn the_state_is_heavy_cmd_home_else_xdg_state_home_else_local_state() {
    let home = TempDir::new("heavy-cmd-config");
    let env = env_in(&home);
    assert_eq!(state_dir(&env), home.join(".local/state/heavy-cmd"));
    assert_eq!(state_dir(&env.clone().with("XDG_STATE_HOME", "/xdg/state")), Path::new("/xdg/state/heavy-cmd"));
    assert_eq!(state_dir(&env.with("XDG_STATE_HOME", "/xdg/state").with("HEAVY_CMD_HOME", "/heavy")), Path::new("/heavy"));
}

#[cfg(unix)]
#[test]
fn the_config_is_xdg_config_home_else_dot_config_and_a_relative_xdg_folder_is_ignored() {
    let home = TempDir::new("heavy-cmd-config");
    let env = env_in(&home);
    assert_eq!(config_file(&env), home.join(".config/heavy-cmd/config"));
    assert_eq!(config_file(&env.clone().with("XDG_CONFIG_HOME", "/xdg/config")), Path::new("/xdg/config/heavy-cmd/config"));
    assert_eq!(config_file(&env.with("XDG_CONFIG_HOME", "relative")), home.join(".config/heavy-cmd/config"));
}

#[cfg(windows)]
#[test]
fn on_windows_the_state_is_in_local_app_data_and_the_config_in_app_data() {
    let home = TempDir::new("heavy-cmd-config");
    let env = env_in(&home);
    assert_eq!(state_dir(&env), home.join("AppData/Local").join("heavy-cmd"));
    assert_eq!(config_file(&env), home.join("AppData/Roaming").join("heavy-cmd").join("config"));
    let env = env.with("LOCALAPPDATA", r"C:\local").with("APPDATA", r"C:\roaming");
    assert_eq!(state_dir(&env), Path::new(r"C:\local\heavy-cmd"));
    assert_eq!(config_file(&env), Path::new(r"C:\roaming\heavy-cmd\config"));
    assert_eq!(state_dir(&env.clone().with("XDG_STATE_HOME", r"C:\xdg")), Path::new(r"C:\xdg\heavy-cmd"));
    assert_eq!(state_dir(&env.with("heavy_cmd_home", r"C:\heavy")), Path::new(r"C:\heavy"));
}

#[test]
fn with_no_config_file_cpu_60_wait_900_auto_update_on() {
    let home = TempDir::new("heavy-cmd-config");
    assert_eq!(load_config(&env_in(&home), None).unwrap(), DEFAULTS);
    assert_eq!(DEFAULTS, Config { cpu: 60, wait: 900, auto_update: true });
}

#[test]
fn the_files_lines_set_them_comments_and_blanks_skipped_the_last_repeat_counts() {
    let home = TempDir::new("heavy-cmd-config");
    let env = env_in(&home);
    config(&env, "# the machine's queue\n\ncpu = 80\nwait=30   # half a minute\r\nauto_update = off\ncpu = 75\n");
    assert_eq!(load_config(&env, None).unwrap(), Config { cpu: 75, wait: 30, auto_update: false });
}

#[test]
fn the_env_overrides_the_file_and_wait_overrides_the_env_an_empty_variable_is_unset() {
    let home = TempDir::new("heavy-cmd-config");
    let env = env_in(&home);
    config(&env, "cpu = 80\nwait = 30\nauto_update = off\n");
    let over = env.clone().with("HEAVY_CMD_CPU", "40").with("HEAVY_CMD_WAIT", "60").with("HEAVY_CMD_AUTO_UPDATE", "on");
    assert_eq!(load_config(&over, None).unwrap(), Config { cpu: 40, wait: 60, auto_update: true });
    assert_eq!(load_config(&over, Some("5")).unwrap(), Config { cpu: 40, wait: 5, auto_update: true });
    assert_eq!(load_config(&env.with("HEAVY_CMD_CPU", ""), None).unwrap().cpu, 80);
}

#[test]
fn the_bounds_are_cpu_10_to_100_and_wait_0_to_86400() {
    let home = TempDir::new("heavy-cmd-config");
    let env = env_in(&home);
    let low = load_config(&env.clone().with("HEAVY_CMD_CPU", "10").with("HEAVY_CMD_WAIT", "0"), None).unwrap();
    assert_eq!((low.cpu, low.wait), (10, 0));
    let high = load_config(&env.clone().with("HEAVY_CMD_CPU", "100").with("HEAVY_CMD_WAIT", "86400"), None).unwrap();
    assert_eq!((high.cpu, high.wait), (100, 86_400));
    assert_eq!(refused(&env.clone().with("HEAVY_CMD_CPU", "9"), None), "HEAVY_CMD_CPU must be an integer between 10 and 100, not \"9\"");
    assert!(refused(&env.clone().with("HEAVY_CMD_CPU", "101"), None).starts_with("HEAVY_CMD_CPU must be an integer between 10 and 100"));
    assert!(refused(&env.clone().with("HEAVY_CMD_CPU", "60.5"), None).starts_with("HEAVY_CMD_CPU must be an integer between 10 and 100"));
    let wait = refused(&env.clone().with("HEAVY_CMD_WAIT", "86401"), None);
    assert_eq!(wait, "HEAVY_CMD_WAIT must be an integer number of seconds between 0 and 86400, not \"86401\"");
    assert_eq!(refused(&env, Some("-1")), "--wait must be an integer number of seconds between 0 and 86400, not \"-1\"");
    assert_eq!(refused(&env.with("HEAVY_CMD_AUTO_UPDATE", "yes"), None), "HEAVY_CMD_AUTO_UPDATE must be on or off, not \"yes\"");
}

#[test]
fn a_bad_file_is_refused_naming_its_line_and_why() {
    let home = TempDir::new("heavy-cmd-config");
    let env = env_in(&home);
    let file = config(&env, "wait = 10\ncpu = 5\n").display().to_string();
    assert_eq!(refused(&env, None), format!("{file}:2: cpu must be an integer between 10 and 100, not \"5\""));
    config(&env, "heavy_cpu = 60\n");
    assert_eq!(refused(&env, None), format!("{file}:1: unknown key heavy_cpu; the keys are cpu, wait, auto_update"));
    config(&env, "cpu 60\n");
    assert_eq!(refused(&env, None), format!("{file}:1: expected key = value, not \"cpu 60\""));
}

#[test]
fn a_refusal_exits_78_sysexits_ex_config() {
    let home = TempDir::new("heavy-cmd-config");
    assert_eq!(load_config(&env_in(&home).with("HEAVY_CMD_CPU", "x"), None).unwrap_err().code(), 78);
}
