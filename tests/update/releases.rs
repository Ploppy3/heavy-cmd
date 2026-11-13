//! `heavy-cmd update` on a fake install whose binaries are scripts that say their version: Linux's,
//! since they are shell scripts, and tar and curl do the work.

use super::common::TempDir;
use super::common::releases::{Releases, fake, release};
use heavy_cmd::config::Env;
use heavy_cmd::fail::{Result, other};
use heavy_cmd::install::{Install, MARKER};
use heavy_cmd::update::{Release, UpdatePorts, github_ports, old_of, update};
use std::cell::RefCell;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::rc::Rc;

/// An install made by install.sh, at `tag`, in `<dir>/share`.
fn installed(dir: &TempDir, tag: &str) -> Install {
    let share = dir.join("share");
    let bin = share.join("bin").join("heavy-cmd");
    std::fs::create_dir_all(bin.parent().unwrap()).unwrap();
    std::fs::write(&bin, fake(tag)).unwrap();
    std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755)).unwrap();
    std::fs::write(share.join(MARKER), "").unwrap();
    Install::Installed { dir: share, bin }
}

/// The releases: the newest is `tag`, its asset `tarball`; what is downloaded goes in `downloads`.
fn ports(tag: &str, tarball: PathBuf, downloads: &Rc<RefCell<Vec<String>>>) -> UpdatePorts {
    let (tag, downloads) = (tag.to_string(), downloads.clone());
    UpdatePorts {
        latest: Box::new(move || Ok(Release { tag: tag.clone(), url: tarball.display().to_string() })),
        download: Box::new(move |url, file| {
            downloads.borrow_mut().push(url.to_string());
            std::fs::copy(url, file).map(|_| ()).map_err(|error| other(error.to_string()))
        }),
    }
}

/// What `bin --version` says.
fn says(bin: &Path) -> String {
    String::from_utf8(Command::new(bin).arg("--version").output().unwrap().stdout).unwrap().trim().to_string()
}

fn bin_of(install: &Install) -> PathBuf {
    match install {
        Install::Installed { bin, .. } => bin.clone(),
        _ => unreachable!(),
    }
}

fn names(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> =
        std::fs::read_dir(dir).unwrap().map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned()).collect();
    names.sort();
    names
}

fn run(ports: &UpdatePorts, check: bool, install: &Install, current: &str, said: &mut Vec<String>) -> Result<()> {
    update(ports, check, install, current, &mut |line| said.push(line.to_string()))
}

#[test]
fn refuses_in_a_dev_checkout_and_anywhere_install_sh_did_not_install() {
    let dir = TempDir::new("heavy-cmd-update");
    let downloads = Rc::default();
    let unused = ports("v9.0.0", "unused".into(), &downloads);
    let dev = run(&unused, false, &Install::Dev { root: dir.path().into() }, "v0.1.0", &mut Vec::new()).unwrap_err().to_string();
    assert!(dev.starts_with(&format!("{} is a dev checkout: update it with git", dir.path().display())), "{dev}");
    let other = run(&unused, false, &Install::Other { exe: dir.path().into() }, "v0.1.0", &mut Vec::new()).unwrap_err().to_string();
    assert!(other.starts_with(&format!("{} is not an install made by install.sh", dir.path().display())), "{other}");
}

#[test]
fn moves_the_install_to_the_newest_release_keeping_the_one_before_it() {
    let dir = TempDir::new("heavy-cmd-update");
    let install = installed(&dir, "v0.1.0");
    let (bin, downloads, mut said) = (bin_of(&install), Rc::default(), Vec::new());
    run(&ports("v0.2.0", release(dir.path(), Some(&fake("v0.2.0"))), &downloads), false, &install, "v0.1.0", &mut said).unwrap();
    assert_eq!(says(&bin), "heavy-cmd v0.2.0");
    run(&ports("v0.3.0", release(dir.path(), Some(&fake("v0.3.0"))), &downloads), false, &install, "v0.2.0", &mut said).unwrap();
    assert_eq!((says(&bin), says(&old_of(&bin))), ("heavy-cmd v0.3.0".into(), "heavy-cmd v0.2.0".into()));
    assert_eq!(names(&dir.join("share").join("bin")), ["heavy-cmd", "heavy-cmd.old"]);
    assert_eq!(names(&dir.join("share")), [MARKER, "bin"]);
    assert_eq!(said, ["heavy-cmd updated from v0.1.0 to v0.2.0", "heavy-cmd updated from v0.2.0 to v0.3.0"]);
}

#[test]
fn never_moves_back_nor_downloads_what_it_has() {
    let dir = TempDir::new("heavy-cmd-update");
    let install = installed(&dir, "v0.2.0");
    let (downloads, mut said) = (Rc::default(), Vec::new());
    run(&ports("v0.1.9", "unused".into(), &downloads), false, &install, "v0.2.0", &mut said).unwrap();
    run(&ports("v0.2.0", "unused".into(), &downloads), false, &install, "v0.2.0", &mut said).unwrap();
    assert!(downloads.borrow().is_empty());
    assert_eq!(
        said,
        ["heavy-cmd v0.2.0 is up to date (the newest release is v0.1.9)", "heavy-cmd v0.2.0 is up to date (the newest release is v0.2.0)"]
    );
}

#[test]
fn check_only_says_whether_a_newer_one_is_out() {
    let dir = TempDir::new("heavy-cmd-update");
    let install = installed(&dir, "v0.1.0");
    let (downloads, mut said) = (Rc::default(), Vec::new());
    run(&ports("v0.10.0", "unused".into(), &downloads), true, &install, "v0.1.0", &mut said).unwrap();
    assert_eq!(said, ["heavy-cmd v0.10.0 is out; this is v0.1.0: heavy-cmd update installs it"]);
    assert!(downloads.borrow().is_empty());
    assert_eq!(says(&bin_of(&install)), "heavy-cmd v0.1.0");
}

#[test]
fn a_new_version_that_does_not_run_or_names_another_is_not_switched_to_and_nothing_of_it_is_left() {
    let dir = TempDir::new("heavy-cmd-update");
    let install = installed(&dir, "v0.1.0");
    let downloads = Rc::default();
    let broken = release(dir.path(), Some("#!/bin/sh\nexit 1\n"));
    let failed = run(&ports("v0.2.0", broken, &downloads), false, &install, "v0.1.0", &mut Vec::new()).unwrap_err().to_string();
    assert!(failed.starts_with("update to v0.2.0 failed, v0.1.0 kept: v0.2.0 does not run here"), "{failed}");
    let elsewhere = release(dir.path(), Some(&fake("v0.2.5")));
    let failed = run(&ports("v0.3.0", elsewhere, &downloads), false, &install, "v0.1.0", &mut Vec::new()).unwrap_err().to_string();
    assert!(failed.ends_with("its --version said \"heavy-cmd v0.2.5\""), "{failed}");
    let offline = UpdatePorts { download: Box::new(|_, _| Err(other("offline"))), ..ports("v0.4.0", "unused".into(), &downloads) };
    let failed = run(&offline, false, &install, "v0.1.0", &mut Vec::new()).unwrap_err().to_string();
    assert_eq!(failed, "update to v0.4.0 failed, v0.1.0 kept: offline");
    assert_eq!(names(&dir.join("share")), [MARKER, "bin"]);
    assert_eq!(names(&dir.join("share").join("bin")), ["heavy-cmd"]);
    assert_eq!(says(&bin_of(&install)), "heavy-cmd v0.1.0");
}

#[test]
fn one_update_at_a_time_a_live_ones_lock_refuses_another_a_dead_ones_is_taken_over() {
    let dir = TempDir::new("heavy-cmd-update");
    let install = installed(&dir, "v0.1.0");
    let downloads = Rc::default();
    let lock = dir.join("share").join("update.lock");
    std::fs::write(&lock, std::process::id().to_string()).unwrap();
    let to = |tag: &str| ports(tag, release(dir.path(), Some(&fake(tag))), &downloads);
    let refused = run(&to("v0.2.0"), false, &install, "v0.1.0", &mut Vec::new()).unwrap_err().to_string();
    assert_eq!(refused, format!("another heavy-cmd update is running (pid {})", std::process::id()));
    std::fs::write(&lock, "999999999").unwrap();
    run(&to("v0.2.0"), false, &install, "v0.1.0", &mut Vec::new()).unwrap();
    assert_eq!(says(&bin_of(&install)), "heavy-cmd v0.2.0");
    assert!(!lock.exists());
}

#[test]
fn reads_the_newest_release_and_downloads_its_asset_over_http_as_from_githubs_api() {
    let dir = TempDir::new("heavy-cmd-update");
    let releases = Releases::serve();
    releases.set("v0.2.0", release(dir.path(), Some(&fake("v0.2.0"))));
    let install = installed(&dir, "v0.1.0");
    let api = |path: &str| github_ports(&Env::of(&[("HEAVY_CMD_RELEASES_API", &format!("{}{path}", releases.base))]));
    run(&api("/latest"), false, &install, "v0.1.0", &mut Vec::new()).unwrap();
    assert_eq!(says(&bin_of(&install)), "heavy-cmd v0.2.0");
    let latest = |path: &str| (api(path).latest)().unwrap_err().to_string();
    assert_eq!(latest("/no-asset"), format!("release v0.2.0 has no {}", heavy_cmd::install::asset()));
    let gone = latest("/gone");
    assert!(gone.starts_with(&format!("{}/gone: ", releases.base)) && gone.contains("404"), "{gone}");
    let refused = (github_ports(&Env::of(&[("HEAVY_CMD_RELEASES_API", "http://127.0.0.1:1/latest")])).latest)().unwrap_err().to_string();
    assert!(refused.starts_with("http://127.0.0.1:1/latest: ") && refused.len() > 30, "{refused}");
}

#[test]
fn the_auto_update_starts_bin_update_detached_its_output_in_the_log() {
    let dir = TempDir::new("heavy-cmd-update");
    let bin = dir.join("heavy-cmd");
    std::fs::write(&bin, "#!/bin/sh\necho \"ran $1 in session $(cut -d' ' -f6 /proc/$$/stat)\"\n").unwrap();
    std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755)).unwrap();
    let log = dir.join("update.log");
    heavy_cmd::update::start_detached(&bin, &log).unwrap();
    super::common::wait_for(|| std::fs::read_to_string(&log).is_ok_and(|text| text.contains("ran update")));
    let text = std::fs::read_to_string(&log).unwrap();
    let own_session = std::fs::read_to_string("/proc/self/stat").unwrap().split(' ').nth(5).unwrap().to_string();
    assert!(!text.contains(&format!("session {own_session}\n")), "not detached: {text}");
}
