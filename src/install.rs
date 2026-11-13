use std::cmp::Ordering;
use std::path::{Path, PathBuf};

/// This build's version, `0.2.0`.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// The file install.sh and install.ps1 leave in the install folder: what tells an install from a binary copied anywhere.
pub const MARKER: &str = ".heavy-cmd-install";

/// The binary's name: `heavy-cmd`, `heavy-cmd.exe` on Windows.
pub fn exe_name() -> String {
    format!("heavy-cmd{}", std::env::consts::EXE_SUFFIX)
}

/// This build's release asset: `heavy-cmd-linux-x86_64.tar.gz`, `heavy-cmd-windows-x86_64.zip`.
pub fn asset() -> String {
    let ext = if cfg!(windows) { "zip" } else { "tar.gz" };
    format!("heavy-cmd-{}-{}.{ext}", std::env::consts::OS, std::env::consts::ARCH)
}

/// What runs: a dev build, under a git clone with its Cargo.toml; an install made by install.sh or
/// install.ps1, `<dir>/bin/heavy-cmd` with `<dir>/.heavy-cmd-install`; or anything else, such as an
/// unpacked release.
#[derive(Debug, Clone, PartialEq)]
pub enum Install {
    Dev { root: PathBuf },
    Installed { dir: PathBuf, bin: PathBuf },
    Other { exe: PathBuf },
}

pub fn install_of(exe: &Path) -> Install {
    if let Some(root) = exe.ancestors().skip(1).find(|dir| dir.join("Cargo.toml").is_file() && dir.join(".git").exists()) {
        return Install::Dev { root: root.to_path_buf() };
    }
    let dir = exe.parent().filter(|bin| bin.file_name().is_some_and(|name| name == "bin")).and_then(Path::parent);
    match dir {
        Some(dir) if dir.join(MARKER).is_file() => Install::Installed { dir: dir.to_path_buf(), bin: exe.to_path_buf() },
        _ => Install::Other { exe: exe.to_path_buf() },
    }
}

/// The binary that runs, as the system names it: on Linux, links resolved.
pub fn this_exe() -> PathBuf {
    std::env::current_exe().unwrap_or_default()
}

/// `heavy-cmd v0.2.0`, ` (dev checkout)` after it in a git clone: what `--version` prints, and what an
/// update checks a new one prints.
pub fn version_line(install: &Install) -> String {
    let dev = if matches!(install, Install::Dev { .. }) { " (dev checkout)" } else { "" };
    format!("heavy-cmd v{VERSION}{dev}")
}

/// `[1, 2, 3]` of `v1.2.3` or `1.2.3`; None for anything that is not a release version.
pub fn parse_version(text: &str) -> Option<[u64; 3]> {
    let text = text.trim();
    let parts: Vec<&str> = text.strip_prefix('v').unwrap_or(text).split('.').collect();
    let [a, b, c] = parts.as_slice() else { return None };
    let number = |part: &str| (!part.is_empty() && part.bytes().all(|b| b.is_ascii_digit())).then(|| part.parse().ok()).flatten();
    Some([number(a)?, number(b)?, number(c)?])
}

/// How release version `a` compares to `b`; else which is not one.
pub fn compare_versions(a: &str, b: &str) -> Result<Ordering, String> {
    match (parse_version(a), parse_version(b)) {
        (Some(x), Some(y)) => Ok(x.cmp(&y)),
        (None, _) => Err(format!("not a release version: {a}")),
        (_, None) => Err(format!("not a release version: {b}")),
    }
}
