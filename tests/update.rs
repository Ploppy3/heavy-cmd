//! The self-update: SHA-256, reading the latest tag and SHA256SUMS, moving a binary to a newer release
//! only once its SHA-256 matches and it runs, against fake releases; and the hourly check's turn.

mod common;

use common::{TempDir, World, bin};
use heavy_cmd::fail::{Fail, Result};
use heavy_cmd::sha256::sha256_hex;
use heavy_cmd::update::{CHECK_EVERY_MS, SUMS, UpdatePorts, asset, parse_version, sum_of, tag_of, update};
use std::path::{Path, PathBuf};

const VERSION: &str = env!("CARGO_PKG_VERSION");

#[test]
fn sha256_gives_the_standard_digests() {
    assert_eq!(sha256_hex(b""), "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855");
    assert_eq!(sha256_hex(b"abc"), "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad");
    let two_blocks = b"abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq";
    assert_eq!(sha256_hex(two_blocks), "248d6a61d20638b8e5c026930c3e6039a33ce45964ff2167f6ecedd419db06c1");
    assert_eq!(sha256_hex(&vec![b'a'; 1_000_000]), "cdc76e5c9914fb9281a1c7e284d73e67f1809a48a497200e046d39ccc7112cd0");
    // The longest data whose padding fits its block, and a whole block, whose padding takes one more.
    assert_eq!(sha256_hex(&[0; 55]), "02779466cdec163811d078815c633f21901413081449002f24aa3e80f0b88ef7");
    assert_eq!(sha256_hex(&[0; 64]), "f5a5fd42d16a20302798ef6ed309979b43003d2320d9f0e8ea9831a92759fb4b");
}

#[cfg(unix)]
#[test]
fn sha256_of_the_binary_is_sha256sum_s() {
    let out = std::process::Command::new("sha256sum").arg(bin()).output().unwrap();
    let want = String::from_utf8_lossy(&out.stdout).split_whitespace().next().unwrap().to_string();
    assert_eq!(sha256_hex(&std::fs::read(bin()).unwrap()), want);
}

#[test]
fn the_latest_tag_is_read_from_the_redirect() {
    let headers = "HTTP/2 302 \r\ndate: x\r\nLocation: https://github.com/Ploppy3/heavy-cmd/releases/tag/v0.4.0\r\n\r\n";
    assert_eq!(tag_of(headers), Some("v0.4.0".into()));
    assert_eq!(tag_of("HTTP/2 302 \r\nlocation: https://github.com/Ploppy3/heavy-cmd/releases\r\n"), None);
    assert_eq!(tag_of("HTTP/2 200 \r\n"), None);
    assert_eq!((parse_version("v1.20.3"), parse_version("1.2"), parse_version("v1.2.x")), (Some([1, 20, 3]), None, None));
}

#[test]
fn sha256sums_names_each_file_s_sum() {
    let a = "a".repeat(64);
    let sums = format!("{a}  heavy-cmd-linux-x86_64\n{}  *heavy-cmd-windows-x86_64.exe\n", "B".repeat(64));
    assert_eq!(sum_of(&sums, "heavy-cmd-linux-x86_64"), Some(a));
    assert_eq!(sum_of(&sums, "heavy-cmd-windows-x86_64.exe"), Some("b".repeat(64)));
    assert_eq!(sum_of(&sums, "heavy-cmd"), None);
    assert_eq!(sum_of("abc  heavy-cmd-linux-x86_64\n", "heavy-cmd-linux-x86_64"), None);
}

/// An install of a binary that says "old", in a folder of its own.
struct Install {
    dir: TempDir,
    bin: PathBuf,
}

impl Install {
    fn new() -> Install {
        let dir = TempDir::new("heavy-cmd-update");
        let bin = dir.join(&format!("heavy-cmd{}", std::env::consts::EXE_SUFFIX));
        std::fs::write(&bin, "old").unwrap();
        Install { dir, bin }
    }

    /// What is left in its folder: only the binary, after any update.
    fn files(&self) -> Vec<String> {
        let mut names: Vec<String> =
            std::fs::read_dir(self.dir.path()).unwrap().map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned()).collect();
        names.sort();
        names
    }

    fn update(&self, current: &str, ports: &UpdatePorts) -> (Result<()>, Vec<String>) {
        let mut said = Vec::new();
        let done = update(ports, current, &self.bin, &mut |line| said.push(line.to_string()));
        (done, said)
    }
}

/// A release `tag` whose binary is `binary` and whose SHA256SUMS is `sums`.
fn release(tag: &str, binary: Vec<u8>, sums: String) -> UpdatePorts {
    let tag = tag.to_string();
    let latest = tag.clone();
    UpdatePorts {
        latest: Box::new(move || Ok(latest.clone())),
        fetch: Box::new(move |at, name| {
            assert_eq!(at, tag);
            match name {
                SUMS => Ok(sums.clone().into_bytes()),
                name if name == asset() => Ok(binary.clone()),
                name => Err(Fail::Other(format!("no {name}"))),
            }
        }),
    }
}

/// The binary under test as the release's, with its true SHA-256: it names this version.
fn true_release(tag: &str) -> (UpdatePorts, Vec<u8>) {
    let binary = std::fs::read(bin()).unwrap();
    let sums = format!("{}  {}\n{}  heavy-cmd-other\n", sha256_hex(&binary), asset(), "0".repeat(64));
    (release(tag, binary.clone(), sums), binary)
}

fn binary_name(path: &Path) -> String {
    path.file_name().unwrap().to_string_lossy().into_owned()
}

#[test]
fn a_newer_release_whose_binary_matches_its_sum_and_runs_takes_the_binary_s_place() {
    let install = Install::new();
    let (ports, binary) = true_release(&format!("v{VERSION}"));
    let (done, said) = install.update("0.0.1", &ports);
    done.unwrap();
    assert_eq!(said, [format!("heavy-cmd updated from v0.0.1 to v{VERSION}")]);
    assert_eq!(std::fs::read(&install.bin).unwrap(), binary);
    assert_eq!(install.files(), [binary_name(&install.bin)]);
}

#[test]
fn the_same_or_an_older_release_is_up_to_date_and_downloads_nothing() {
    let install = Install::new();
    for (current, latest) in [("0.4.0", "v0.4.0"), ("0.10.0", "v0.9.9")] {
        let latest = latest.to_string();
        let ports = UpdatePorts { latest: Box::new(move || Ok(latest.clone())), fetch: Box::new(|_, name| panic!("fetched {name}")) };
        let (done, said) = install.update(current, &ports);
        done.unwrap();
        assert!(said[0].starts_with(&format!("heavy-cmd v{current} is up to date")), "{said:?}");
        assert_eq!(std::fs::read(&install.bin).unwrap(), b"old");
    }
}

#[test]
fn a_binary_that_does_not_match_its_sum_is_never_written() {
    let install = Install::new();
    let binary = std::fs::read(bin()).unwrap();
    let mut broken = binary.clone();
    broken.truncate(binary.len() / 2);
    let cases = [
        (broken, format!("{}  {}\n", sha256_hex(&binary), asset()), "does not have the SHA-256 its SHA256SUMS names"),
        (binary.clone(), format!("{}  {}\n", "0".repeat(64), asset()), "does not have the SHA-256 its SHA256SUMS names"),
        (binary.clone(), format!("{}  heavy-cmd-other\n", sha256_hex(&binary)), "its SHA256SUMS names no"),
    ];
    for (served, sums, why) in cases {
        let (done, _) = install.update("0.0.1", &release("v9.0.0", served, sums));
        let error = done.unwrap_err().to_string();
        assert!(error.starts_with("update to v9.0.0 failed, v0.0.1 kept: ") && error.contains(why), "{error}");
        assert_eq!(std::fs::read(&install.bin).unwrap(), b"old");
        assert_eq!(install.files(), [binary_name(&install.bin)]);
    }
}

#[test]
fn a_binary_that_does_not_name_its_release_when_run_is_not_put_in_place() {
    let install = Install::new();
    let (ports, _) = true_release("v99.0.0");
    let (done, _) = install.update("0.0.1", &ports);
    let error = done.unwrap_err().to_string();
    assert!(error.contains(&format!("it does not run here: it said \"heavy-cmd v{VERSION}\"")), "{error}");
    assert_eq!(std::fs::read(&install.bin).unwrap(), b"old");
    assert_eq!(install.files(), [binary_name(&install.bin)]);
}

#[test]
fn a_release_that_cannot_be_read_leaves_the_binary_as_it_was() {
    let install = Install::new();
    let ports =
        UpdatePorts { latest: Box::new(|| Err(Fail::Other("offline".into()))), fetch: Box::new(|_, name| panic!("fetched {name}")) };
    assert_eq!(install.update("0.0.1", &ports).0.unwrap_err().to_string(), "offline");
    let ports = UpdatePorts { latest: Box::new(|| Ok("v9.0.0".into())), fetch: Box::new(|_, _| Err(Fail::Other("404".into()))) };
    assert_eq!(install.update("0.0.1", &ports).0.unwrap_err().to_string(), "update to v9.0.0 failed, v0.0.1 kept: 404");
    assert_eq!(install.files(), [binary_name(&install.bin)]);
}

#[test]
fn the_check_is_due_once_an_hour_and_one_heavy_cmd_takes_it() {
    let world = World::new();
    let (a, b) = (world.heavy(), world.heavy());
    assert!(a.update_due(CHECK_EVERY_MS).unwrap());
    assert!(!b.update_due(CHECK_EVERY_MS).unwrap());
    world.advance(CHECK_EVERY_MS - 1);
    assert!(!a.update_due(CHECK_EVERY_MS).unwrap());
    world.advance(1);
    assert!(b.update_due(CHECK_EVERY_MS).unwrap());
    assert!(!a.update_due(CHECK_EVERY_MS).unwrap());
    // A clock set back does not hold the next check off for the time it went back.
    world.advance(-10 * CHECK_EVERY_MS);
    assert!(a.update_due(CHECK_EVERY_MS).unwrap());
}
