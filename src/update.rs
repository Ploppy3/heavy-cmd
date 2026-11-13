use crate::config::{Env, load_config};
use crate::fail::{Result, other};
use crate::format::{iso, parse_iso};
use crate::install::{Install, asset, compare_versions, exe_name, parse_version};
use crate::sys;
use std::cmp::Ordering;
use std::fs::{self, File, OpenOptions};
use std::io::{ErrorKind, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// Where the newest release is read from; `HEAVY_CMD_RELEASES_API` points elsewhere, a mirror or a test's server.
pub const RELEASES_API: &str = "https://api.github.com/repos/Ploppy3/heavy-cmd/releases/latest";

/// A release by its tag, `v0.2.0`, and the URL of this build's asset in it: None when it has none,
/// as a release before this system's builds.
#[derive(Debug, Clone, PartialEq)]
pub struct Release {
    pub tag: String,
    pub url: Option<String>,
}

/// Reads the newest release.
pub type Latest = Box<dyn Fn() -> Result<Release>>;
/// Downloads a URL into a file.
pub type Download = Box<dyn Fn(&str, &Path) -> Result<()>>;

/// Where releases come from: GitHub's, or a test's.
pub struct UpdatePorts {
    pub latest: Latest,
    pub download: Download,
}

/// One of Windows' own tools (`curl.exe`, `tar.exe`), not another on PATH, such as Git's GNU tar,
/// which reads no zip; on Linux, PATH's.
fn tool(name: &str) -> PathBuf {
    if cfg!(windows)
        && let Some(root) = std::env::var_os("SystemRoot")
    {
        let own = Path::new(&root).join("System32").join(format!("{name}.exe"));
        if own.is_file() {
            return own;
        }
    }
    PathBuf::from(name)
}

/// What `command` writes once it ends, or why it failed: its stderr, else how it ended.
fn output(mut command: Command, what: &str) -> Result<Vec<u8>> {
    let done = command.stdin(Stdio::null()).output().map_err(|error| other(format!("{what}: {error}")))?;
    if done.status.success() {
        return Ok(done.stdout);
    }
    let stderr = String::from_utf8_lossy(&done.stderr);
    // curl says `curl: (22) The requested URL returned error: 404`: the reason is after its number.
    let why = stderr.trim().trim_start_matches("curl: ").trim_start_matches(|c: char| c == '(' || c.is_ascii_digit() || c == ')').trim();
    Err(other(format!("{what}: {}", if why.is_empty() { done.status.to_string() } else { why.to_string() })))
}

fn curl(url: &str, timeout_sec: u32) -> Command {
    let mut command = Command::new(tool("curl"));
    command.args([
        "-fsSL",
        "--max-time",
        &timeout_sec.to_string(),
        "-H",
        "User-Agent: heavy-cmd",
        "-H",
        "Accept: application/vnd.github+json",
    ]);
    command.arg(url);
    command
}

/// The newest release, as GitHub's `releases/latest` gives it: its tag and this build's asset.
pub fn latest_release(api: &str) -> Result<Release> {
    let body = output(curl(api, 30), api)?;
    let release: serde_json::Value = serde_json::from_slice(&body).map_err(|error| other(format!("{api}: {error}")))?;
    let tag = release["tag_name"].as_str().unwrap_or("").to_string();
    if parse_version(&tag).is_none() {
        return Err(other(format!("{api}: no release tag, read \"{tag}\"")));
    }
    let name = asset();
    let assets = release["assets"].as_array().map(Vec::as_slice).unwrap_or_default();
    let url = assets.iter().find(|each| each["name"] == name.as_str()).and_then(|each| each["browser_download_url"].as_str());
    Ok(Release { tag, url: url.map(str::to_string) })
}

/// The releases on GitHub, or the API `HEAVY_CMD_RELEASES_API` names, through curl.
pub fn github_ports(env: &Env) -> UpdatePorts {
    let api = env.get("HEAVY_CMD_RELEASES_API").unwrap_or(RELEASES_API).to_string();
    UpdatePorts {
        latest: Box::new(move || latest_release(&api)),
        download: Box::new(|url, file| {
            let mut command = curl(url, 300);
            command.arg("-o").arg(file);
            output(command, url).map(|_| ())
        }),
    }
}

/// What `bin --version` prints, or why it did not run; it gets 30 seconds.
fn version_of(bin: &Path) -> String {
    let child = Command::new(bin).arg("--version").stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn();
    let mut child = match child {
        Ok(child) => child,
        Err(error) => return format!("failed: {error}"),
    };
    let started = Instant::now();
    while matches!(child.try_wait(), Ok(None)) {
        if started.elapsed() > Duration::from_secs(30) {
            let _ = child.kill();
            break;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    match child.wait_with_output() {
        Ok(done) if done.status.success() => String::from_utf8_lossy(&done.stdout).trim().to_string(),
        Ok(done) => {
            let stderr = String::from_utf8_lossy(&done.stderr).trim().to_string();
            format!("failed: {}", if stderr.is_empty() { done.status.to_string() } else { stderr })
        }
        Err(error) => format!("failed: {error}"),
    }
}

/// `work` while this process holds `<dir>/update.lock`: two updates never switch at once. One left by a dead process is taken over.
fn locked<T>(dir: &Path, work: impl FnOnce() -> Result<T>) -> Result<T> {
    let lock = dir.join("update.lock");
    let mut tries = 0;
    loop {
        match OpenOptions::new().write(true).create_new(true).open(&lock) {
            Ok(mut file) => {
                file.write_all(std::process::id().to_string().as_bytes())?;
                break;
            }
            Err(error) if error.kind() == ErrorKind::AlreadyExists && tries == 0 => {
                let pid: u32 = fs::read_to_string(&lock).unwrap_or_default().trim().parse().unwrap_or(0);
                if pid > 0 && sys::pid_alive(pid) {
                    return Err(other(format!("another heavy-cmd update is running (pid {pid})")));
                }
                let _ = fs::remove_file(&lock);
                tries += 1;
            }
            Err(error) => return Err(other(format!("{}: {error}", lock.display()))),
        }
    }
    let done = work();
    let _ = fs::remove_file(&lock);
    done
}

/// Unpacks `archive`, a release's `heavy-cmd/` folder, into `into`: tar reads both the tar.gz and the zip.
fn unpack(archive: &Path, into: &Path) -> Result<()> {
    let mut command = Command::new(tool("tar"));
    command.arg(if cfg!(windows) { "-xf" } else { "-xzf" }).arg(archive).arg("-C").arg(into).arg("--strip-components=1");
    output(command, &format!("cannot unpack {}", archive.display())).map(|_| ())
}

/// The binary the one in use was before: `heavy-cmd.old`, `heavy-cmd.old.exe` on Windows.
pub fn old_of(bin: &Path) -> PathBuf {
    bin.with_file_name(format!("heavy-cmd.old{}", std::env::consts::EXE_SUFFIX))
}

/// Downloads and unpacks `release` beside the binary in use in `dir`, and only once its own `--version`
/// names it puts it in `bin`'s place, keeping the one before. A failure leaves `bin` as it was.
fn install_release(dir: &Path, bin: &Path, release: &Release, url: &str, ports: &UpdatePorts) -> Result<()> {
    let work = dir.join(format!(".update-{}-{}", release.tag, std::process::id()));
    let _ = fs::remove_dir_all(&work);
    let unpacked = work.join("heavy-cmd");
    fs::create_dir_all(&unpacked)?;
    let done = (|| {
        let archive = work.join(asset());
        (ports.download)(url, &archive)?;
        unpack(&archive, &unpacked)?;
        let new = unpacked.join("bin").join(exe_name());
        let said = version_of(&new);
        if said != format!("heavy-cmd {}", release.tag) {
            return Err(other(format!("{} does not run here: its --version said \"{said}\"", release.tag)));
        }
        sys::switch(bin, &new, &old_of(bin)).map_err(|error| other(format!("cannot put it in place: {error}")))
    })();
    let _ = fs::remove_dir_all(&work);
    done
}

/// `heavy-cmd update [--check]`: moves `install` from `current`, its version, to the newest release,
/// never back to an older one; `check` only says whether a newer one is out. A dev build, or anything
/// install.sh or install.ps1 did not make, is refused; so is a newer release with no asset for this system.
pub fn update(ports: &UpdatePorts, check: bool, install: &Install, current: &str, say: &mut dyn FnMut(&str)) -> Result<()> {
    let (dir, bin) = match install {
        Install::Dev { root } => {
            let why = "update it with git; heavy-cmd update moves only an install made by install.sh or install.ps1";
            return Err(other(format!("{} is a dev checkout: {why}", root.display())));
        }
        Install::Other { exe } => {
            return Err(other(format!("{} is not an install made by install.sh or install.ps1: nothing to update", exe.display())));
        }
        Install::Installed { dir, bin } => (dir, bin),
    };
    let release = (ports.latest)()?;
    if compare_versions(&release.tag, current).map_err(other)? != Ordering::Greater {
        say(&format!("heavy-cmd {current} is up to date (the newest release is {})", release.tag));
        return Ok(());
    }
    let url = release.url.as_deref().ok_or_else(|| other(format!("release {} has no {}", release.tag, asset())))?;
    if check {
        say(&format!("heavy-cmd {} is out; this is {current}: heavy-cmd update installs it", release.tag));
        return Ok(());
    }
    locked(dir, || {
        install_release(dir, bin, &release, url, ports)
            .map_err(|error| other(format!("update to {} failed, {current} kept: {error}", release.tag)))
    })?;
    say(&format!("heavy-cmd updated from {current} to {}", release.tag));
    Ok(())
}

const DAY_MS: i64 = 86_400_000;

/// Starts `bin update` detached, its output in `log`, and does not wait for it.
pub fn start_detached(bin: &Path, log: &Path) -> std::io::Result<()> {
    let log = File::create(log)?;
    let command = |detach: fn(&mut Command)| -> std::io::Result<()> {
        let mut command = Command::new(bin);
        command.arg("update").stdin(Stdio::null()).stdout(log.try_clone()?).stderr(log.try_clone()?);
        detach(&mut command);
        command.spawn().map(|_| ())
    };
    #[cfg(windows)]
    return command(sys::detach).or_else(|_| command(sys::detach_in_job));
    #[cfg(not(windows))]
    command(sys::detach)
}

/// After a command ends: at most once a day, `heavy-cmd update` detached in the background (`start`),
/// its output in `<install>/update.log`. Never in a dev build, nor with `auto_update = off`; whatever
/// fails is ignored, so the command's output and exit stay its own. Whether it started one.
pub fn auto_update(env: &Env, install: &Install, now: i64, start: &mut dyn FnMut(&Path, &Path) -> std::io::Result<()>) -> bool {
    let Install::Installed { dir, bin } = install else { return false };
    if !load_config(env, None).is_ok_and(|config| config.auto_update) {
        return false;
    }
    let stamp = dir.join("update-check");
    let last = fs::read_to_string(&stamp).ok().and_then(|text| parse_iso(text.trim()));
    if last.is_some_and(|last| (now - last).abs() < DAY_MS) {
        return false;
    }
    fs::write(&stamp, format!("{}\n", iso(now))).is_ok() && start(bin, &dir.join("update.log")).is_ok()
}
