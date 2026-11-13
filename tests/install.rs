//! install.sh end to end against a local release server: Linux's installer.
#![cfg(unix)]

mod common;

use common::TempDir;
use common::releases::{Releases, fake, release};
use heavy_cmd::install::{MARKER, VERSION};
use std::path::Path;
use std::process::{Command, Output};

struct Setup {
    dir: TempDir,
    releases: Releases,
}

impl Setup {
    fn new() -> Setup {
        Setup { dir: TempDir::new("heavy-cmd-install"), releases: Releases::serve() }
    }

    fn command(&self, file: &Path) -> Command {
        let mut command = Command::new(file);
        command.env("HEAVY_CMD_RELEASES_API", self.releases.api()).env("HEAVY_CMD_INSTALL_DIR", self.dir.join("share"));
        command.env("HEAVY_CMD_BIN_DIR", self.dir.join("bin")).env("HEAVY_CMD_HOME", self.dir.join("state"));
        command.env("XDG_CONFIG_HOME", self.dir.join("config")).env("HEAVY_CMD_AUTO_UPDATE", "off").env_remove("HEAVY_CMD_ID");
        command
    }

    fn install_sh(&self) -> Output {
        self.command(Path::new("sh")).arg(Path::new(env!("CARGO_MANIFEST_DIR")).join("install.sh")).output().unwrap()
    }

    fn heavy_cmd(&self, args: &[&str]) -> Output {
        self.command(&self.dir.join("bin").join("heavy-cmd")).current_dir(self.dir.path()).args(args).output().unwrap()
    }
}

fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

/// The version after this build's.
fn next() -> String {
    let [major, minor, _] = heavy_cmd::install::parse_version(VERSION).unwrap();
    format!("v{major}.{}.0", minor + 1)
}

#[test]
fn installs_the_newest_release_into_the_two_folders_and_again_changes_nothing() {
    let setup = Setup::new();
    let tag = format!("v{VERSION}");
    setup.releases.set(&tag, release(setup.dir.path(), None));
    let link = setup.dir.join("bin").join("heavy-cmd");
    let out = setup.install_sh();
    assert_eq!(
        (out.status.code(), text(&out.stdout)),
        (Some(0), format!("heavy-cmd {tag} installed: {}\n", link.display())),
        "{}",
        text(&out.stderr)
    );
    assert_eq!(std::fs::read_link(&link).unwrap(), setup.dir.join("share").join("bin").join("heavy-cmd"));
    assert!(setup.dir.join("share").join(MARKER).is_file());
    assert_eq!(text(&setup.heavy_cmd(&["--version"]).stdout), format!("heavy-cmd {tag}\n"));
    let again = setup.install_sh();
    assert_eq!(again.status.code(), Some(0));
    assert!(text(&again.stderr).contains(&format!("{tag} is already installed")), "{}", text(&again.stderr));
    assert_eq!(setup.heavy_cmd(&["--", "true"]).status.code(), Some(0));
}

#[test]
fn what_it_installs_updates_itself_update_check_then_update() {
    let setup = Setup::new();
    let tag = format!("v{VERSION}");
    setup.releases.set(&tag, release(setup.dir.path(), None));
    assert_eq!(setup.install_sh().status.code(), Some(0));
    let check = setup.heavy_cmd(&["update", "--check"]);
    assert_eq!(text(&check.stdout), format!("heavy-cmd {tag} is up to date (the newest release is {tag})\n"), "{}", text(&check.stderr));
    let next = next();
    setup.releases.set(&next, release(setup.dir.path(), Some(&fake(&next))));
    assert_eq!(
        text(&setup.heavy_cmd(&["update", "--check"]).stdout),
        format!("heavy-cmd {next} is out; this is {tag}: heavy-cmd update installs it\n")
    );
    let update = setup.heavy_cmd(&["update"]);
    assert_eq!(
        (update.status.code(), text(&update.stdout)),
        (Some(0), format!("heavy-cmd updated from {tag} to {next}\n")),
        "{}",
        text(&update.stderr)
    );
    assert_eq!(text(&setup.heavy_cmd(&["--version"]).stdout), format!("heavy-cmd {next}\n"));
    let old = setup.dir.join("share").join("bin").join("heavy-cmd.old");
    assert_eq!(text(&Command::new(old).arg("--version").output().unwrap().stdout), format!("heavy-cmd {tag}\n"));
}

#[test]
fn a_release_that_does_not_run_is_not_installed_and_a_file_in_the_links_place_is_left_alone() {
    let setup = Setup::new();
    let tag = format!("v{VERSION}");
    setup.releases.set(&tag, release(setup.dir.path(), Some("#!/bin/sh\nexit 1\n")));
    let out = setup.install_sh();
    assert_eq!(out.status.code(), Some(1));
    assert!(text(&out.stderr).contains(&format!("{tag} does not run here")), "{}", text(&out.stderr));
    assert!(!setup.dir.join("share").join("bin").join("heavy-cmd").exists());
    setup.releases.set(&tag, release(setup.dir.path(), None));
    std::fs::create_dir_all(setup.dir.join("bin")).unwrap();
    std::fs::write(setup.dir.join("bin").join("heavy-cmd"), "mine").unwrap();
    let out = setup.install_sh();
    assert_eq!(out.status.code(), Some(1));
    assert!(text(&out.stderr).contains("exists and is not a link"), "{}", text(&out.stderr));
    assert_eq!(std::fs::read_to_string(setup.dir.join("bin").join("heavy-cmd")).unwrap(), "mine");
}
