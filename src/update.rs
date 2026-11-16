//! The self-update: a release's binary moves to the newest release, checked at most once an hour after
//! a command ends, and `heavy-cmd update` does it at once. The new binary is put in place only once its
//! SHA-256 is the one the release's SHA256SUMS names, and once it runs and names that release.

use crate::fail::{Fail, Result};
use crate::queue::Heavy;
use crate::sha256::sha256_hex;
use crate::sys;
use std::cmp::Ordering;
use std::fs::File;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// Whether this build updates itself: only a release's binary does (scripts/pack builds them with it).
pub const SELF_UPDATE: bool = cfg!(feature = "self-update");

/// How often, at most, a command's end checks for a newer release.
pub const CHECK_EVERY_MS: i64 = 3_600_000;

/// Where the releases are: `latest` redirects to the newest one's tag, `download/<tag>/<name>` is its files.
const RELEASES: &str = "https://github.com/Ploppy3/heavy-cmd/releases";

/// The release file naming each binary's SHA-256, as `sha256sum` writes them.
pub const SUMS: &str = "SHA256SUMS";

/// This system's binary in a release.
pub fn asset() -> &'static str {
    if cfg!(windows) { "heavy-cmd-windows-x86_64.exe" } else { "heavy-cmd-linux-x86_64" }
}

/// The newest release's tag, `v0.4.0`.
pub type Latest = Box<dyn Fn() -> Result<String>>;
/// A file of release `tag`, by its name: `fetch(tag, name)`.
pub type Fetch = Box<dyn Fn(&str, &str) -> Result<Vec<u8>>>;

/// Where releases come from: GitHub's, or a test's.
pub struct UpdatePorts {
    pub latest: Latest,
    pub fetch: Fetch,
}

impl UpdatePorts {
    /// GitHub's releases, through the system's curl.
    pub fn github() -> UpdatePorts {
        UpdatePorts {
            latest: Box::new(|| {
                let url = format!("{RELEASES}/latest");
                let headers = curl(&url, "-I", 30)?;
                tag_of(&String::from_utf8_lossy(&headers)).ok_or_else(|| Fail::Other(format!("{url}: no release tag in its redirect")))
            }),
            fetch: Box::new(|tag, name| curl(&format!("{RELEASES}/download/{tag}/{name}"), "-L", 300)),
        }
    }
}

/// What curl gets of `url`, over https only, within `timeout_sec`; Windows' own curl.exe there.
fn curl(url: &str, how: &str, timeout_sec: u32) -> Result<Vec<u8>> {
    let mut program = PathBuf::from("curl");
    if cfg!(windows)
        && let Some(root) = std::env::var_os("SystemRoot")
    {
        program = Path::new(&root).join("System32").join("curl.exe");
    }
    let mut command = Command::new(program);
    command.args(["-fsS", "--proto", "=https", "--max-time", &timeout_sec.to_string(), how, url]).stdin(Stdio::null());
    let done = command.output().map_err(|error| Fail::Other(format!("cannot run curl: {error}")))?;
    if done.status.success() {
        return Ok(done.stdout);
    }
    let why = String::from_utf8_lossy(&done.stderr).trim().to_string();
    Err(Fail::Other(format!("{url}: {}", if why.is_empty() { done.status.to_string() } else { why })))
}

/// The tag `releases/latest` redirects to, from its response's headers.
pub fn tag_of(headers: &str) -> Option<String> {
    let location = headers.lines().find_map(|line| {
        let (key, value) = line.split_once(':')?;
        key.trim().eq_ignore_ascii_case("location").then(|| value.trim())
    })?;
    let tag = location.trim_end_matches('/').rsplit('/').next()?;
    parse_version(tag).map(|_| tag.to_string())
}

/// `[1, 2, 3]` of `v1.2.3` or `1.2.3`; None for anything that is not a release version.
pub fn parse_version(text: &str) -> Option<[u64; 3]> {
    let parts: Vec<&str> = text.strip_prefix('v').unwrap_or(text).split('.').collect();
    let number = |part: &str| (!part.is_empty() && part.bytes().all(|b| b.is_ascii_digit())).then(|| part.parse().ok()).flatten();
    match parts.as_slice() {
        [a, b, c] => Some([number(a)?, number(b)?, number(c)?]),
        _ => None,
    }
}

/// The SHA-256 SHA256SUMS names for `name`, in lowercase hex.
pub fn sum_of(sums: &str, name: &str) -> Option<String> {
    sums.lines().find_map(|line| {
        let mut fields = line.split_whitespace();
        let (sum, file) = (fields.next()?, fields.next()?);
        let ok = file.trim_start_matches('*') == name && sum.len() == 64 && sum.bytes().all(|b| b.is_ascii_hexdigit());
        ok.then(|| sum.to_ascii_lowercase())
    })
}

/// The first line `program` says when run with nothing to run, up to its `:`: its usage's, which names
/// its version, `heavy-cmd v0.4.0`. It gets 30 seconds.
fn says(program: &Path) -> String {
    let started = Command::new(program).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::piped()).spawn();
    let mut child = match started {
        Ok(child) => child,
        Err(error) => return format!("nothing, it did not start: {error}"),
    };
    let began = Instant::now();
    while matches!(child.try_wait(), Ok(None)) {
        if began.elapsed() > Duration::from_secs(30) {
            let _ = child.kill();
            break;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    let stderr = child.wait_with_output().map(|done| String::from_utf8_lossy(&done.stderr).into_owned()).unwrap_or_default();
    stderr.lines().next().unwrap_or("").split(':').next().unwrap_or("").trim().to_string()
}

/// Moves `bin`, of version `current`, to the newest release when it is newer, never to an older one.
/// The new binary is put in `bin`'s place only once its SHA-256 is the one SHA256SUMS names and it runs
/// and names its release; else `bin` stays as it was.
pub fn update(ports: &UpdatePorts, current: &str, bin: &Path, say: &mut dyn FnMut(&str)) -> Result<()> {
    let tag = (ports.latest)()?;
    let newer = parse_version(&tag).zip(parse_version(current)).is_some_and(|(latest, this)| latest.cmp(&this) == Ordering::Greater);
    if !newer {
        say(&format!("heavy-cmd v{current} is up to date (the newest release is {tag})"));
        return Ok(());
    }
    let failed = |why: String| Fail::Other(format!("update to {tag} failed, v{current} kept: {why}"));
    let sums = (ports.fetch)(&tag, SUMS).map_err(|fail| failed(fail.to_string()))?;
    let want = sum_of(&String::from_utf8_lossy(&sums), asset()).ok_or_else(|| failed(format!("its {SUMS} names no {}", asset())))?;
    let binary = (ports.fetch)(&tag, asset()).map_err(|fail| failed(fail.to_string()))?;
    if sha256_hex(&binary) != want {
        return Err(failed(format!("its {} does not have the SHA-256 its {SUMS} names", asset())));
    }
    let dir = bin.parent().unwrap_or(Path::new("."));
    let new = dir.join(format!(".heavy-cmd-update-{}{}", std::process::id(), std::env::consts::EXE_SUFFIX));
    let placed = sys::write_program(&new, &binary).map_err(|error| failed(format!("{}: {error}", new.display()))).and_then(|()| {
        let said = says(&new);
        if said != format!("heavy-cmd {tag}") {
            return Err(failed(format!("it does not run here: it said \"{said}\"")));
        }
        sys::replace(bin, &new).map_err(|error| failed(format!("cannot put it in place of {}: {error}", bin.display())))
    });
    let _ = std::fs::remove_file(&new);
    placed?;
    say(&format!("heavy-cmd updated from v{current} to {tag}"));
    Ok(())
}

/// After a command ends, in a release's binary: at most once an hour, `heavy-cmd update` in the
/// background, its output in `<state>/update.log`. Whatever fails is ignored, so the command's output
/// and exit stay its own.
pub fn auto_update(heavy: &Heavy, state: &Path) {
    if SELF_UPDATE && heavy.update_due(CHECK_EVERY_MS).unwrap_or(false) {
        let _ = start(&state.join("update.log"));
    }
}

fn start(log: &Path) -> std::io::Result<()> {
    let log = File::create(log)?;
    let mut command = Command::new(std::env::current_exe()?);
    command.arg("update").stdin(Stdio::null()).stdout(log.try_clone()?).stderr(log);
    sys::spawn_detached(&mut command)
}
