mod common;

use common::TempDir;
use heavy_cmd::install::{Install, compare_versions, install_of};
use heavy_cmd::sys::switch;
use heavy_cmd::update::{auto_update, old_of};
use std::cmp::Ordering;
use std::path::{Path, PathBuf};

#[test]
fn versions_compare_by_number_not_as_text() {
    assert_eq!(compare_versions("v0.10.0", "v0.9.9"), Ok(Ordering::Greater));
    assert_eq!(compare_versions("v1.0.0", "1.0.0"), Ok(Ordering::Equal));
    assert_eq!(compare_versions("v1.2.3", "v1.3.0"), Ok(Ordering::Less));
    assert_eq!(compare_versions("v1", "v1.0.0"), Err("not a release version: v1".into()));
}

#[test]
fn the_new_binary_takes_the_old_ones_place_which_is_kept_as_the_one_before() {
    let dir = TempDir::new("heavy-cmd-switch");
    let bin = dir.join(&format!("heavy-cmd{}", std::env::consts::EXE_SUFFIX));
    let old = old_of(&bin);
    let put = |text: &str| {
        let new = dir.join("new");
        std::fs::write(&new, text).unwrap();
        switch(&bin, &new, &old).unwrap();
    };
    put("one");
    assert_eq!(std::fs::read_to_string(&bin).unwrap(), "one");
    put("two");
    put("three");
    assert_eq!((std::fs::read_to_string(&bin).unwrap(), std::fs::read_to_string(&old).unwrap()), ("three".into(), "two".into()));
    assert!(!dir.join("new").exists());
}

#[test]
fn this_build_is_a_dev_checkout_and_a_binary_elsewhere_no_install() {
    assert!(matches!(install_of(&common::bin()), Install::Dev { .. }));
    let dir = TempDir::new("heavy-cmd-install-of");
    let bin = dir.join("bin").join("heavy-cmd");
    assert!(matches!(install_of(&bin), Install::Other { .. }));
    std::fs::write(dir.join(".heavy-cmd-install"), "").unwrap();
    assert_eq!(install_of(&bin), Install::Installed { dir: dir.path().to_path_buf(), bin });
}

/// The auto update of an install in `dir`, at `now`, with `env`'s settings: whether it started one, and what.
fn auto(dir: &Path, env: &[(&str, &str)], now: i64, started: &mut Vec<(PathBuf, PathBuf)>) -> bool {
    let install = Install::Installed { dir: dir.to_path_buf(), bin: dir.join("bin").join("heavy-cmd") };
    let config = dir.join("config").to_string_lossy().into_owned();
    let env = heavy_cmd::config::Env::of(&[&[("XDG_CONFIG_HOME", config.as_str())], env].concat());
    auto_update(&env, &install, now, &mut |bin, log| {
        started.push((bin.to_path_buf(), log.to_path_buf()));
        Ok(())
    })
}

#[test]
fn the_auto_update_starts_heavy_cmd_update_its_output_in_update_log_at_most_once_a_day() {
    let dir = TempDir::new("heavy-cmd-auto");
    let mut started = Vec::new();
    let day = 86_400_000;
    let now = heavy_cmd::format::parse_iso("2026-11-12T10:00:00Z").unwrap();
    assert!(auto(dir.path(), &[("HEAVY_CMD_AUTO_UPDATE", "on")], now, &mut started));
    assert_eq!(started, [(dir.join("bin").join("heavy-cmd"), dir.join("update.log"))]);
    assert_eq!(std::fs::read_to_string(dir.join("update-check")).unwrap(), "2026-11-12T10:00:00.000Z\n");
    assert!(!auto(dir.path(), &[("HEAVY_CMD_AUTO_UPDATE", "on")], now + day - 1, &mut started));
    assert!(auto(dir.path(), &[("HEAVY_CMD_AUTO_UPDATE", "on")], now + day, &mut started));
    assert_eq!(started.len(), 2);
}

#[test]
fn never_with_auto_update_off_never_in_a_dev_build_or_elsewhere_and_whatever_fails_is_ignored() {
    let dir = TempDir::new("heavy-cmd-auto");
    let mut started = Vec::new();
    assert!(!auto(dir.path(), &[("HEAVY_CMD_AUTO_UPDATE", "off")], 0, &mut started));
    std::fs::create_dir_all(dir.join("config").join("heavy-cmd")).unwrap();
    std::fs::write(dir.join("config").join("heavy-cmd").join("config"), "auto_update = off\n").unwrap();
    assert!(!auto(dir.path(), &[], 0, &mut started));
    assert!(!auto(dir.path(), &[("HEAVY_CMD_AUTO_UPDATE", "maybe")], 0, &mut started));
    assert!(started.is_empty());
    let env = heavy_cmd::config::Env::of(&[("HEAVY_CMD_AUTO_UPDATE", "on")]);
    let never = &mut |_: &Path, _: &Path| -> std::io::Result<()> { panic!("started") };
    assert!(!auto_update(&env, &Install::Dev { root: dir.path().to_path_buf() }, 0, never));
    assert!(!auto_update(&env, &Install::Other { exe: dir.join("heavy-cmd") }, 0, never));
    let failing = &mut |_: &Path, _: &Path| Err(std::io::Error::other("EAGAIN"));
    assert!(!auto_update(&env.with("XDG_CONFIG_HOME", &dir.join("none").to_string_lossy()), &install(dir.path()), 0, failing));
}

fn install(dir: &Path) -> Install {
    Install::Installed { dir: dir.to_path_buf(), bin: dir.join("bin").join("heavy-cmd") }
}

#[cfg(unix)]
#[path = "update/releases.rs"]
mod releases;
