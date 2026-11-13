use std::path::Path;
use std::process::{Command, Stdio};

/// git's answer in `cwd`, trimmed; None when git fails, is missing or says nothing.
fn git(cwd: &Path, args: &[&str]) -> Option<String> {
    let output = Command::new("git").arg("-C").arg(cwd).args(args).stdin(Stdio::null()).stderr(Stdio::null()).output().ok()?;
    let text = String::from_utf8_lossy(&output.stdout).trim().to_string();
    (output.status.success() && !text.is_empty()).then_some(text)
}

/// A remote's repository name: `git@github.com:o/heavy-cmd.git`, `https://host/group/heavy-cmd/` and `/srv/git/heavy-cmd.git` are `heavy-cmd`.
pub fn repo_name(url: &str) -> Option<String> {
    let trimmed = url.trim().trim_end_matches(['/', '\\']);
    let last = trimmed.rsplit(['/', ':', '\\']).next().unwrap_or("");
    let name = last.strip_suffix(".git").unwrap_or(last);
    (!name.is_empty()).then(|| name.to_string())
}

/// The project a command in `cwd` is recorded under: the repository name of the git `origin` remote,
/// else the git top-level folder's name, else the cwd's. Origin comes first: a worktree's folder is not
/// its project's name.
pub fn project_of(cwd: &Path) -> String {
    if let Some(name) = git(cwd, &["remote", "get-url", "origin"]).and_then(|origin| repo_name(&origin)) {
        return name;
    }
    let top = git(cwd, &["rev-parse", "--show-toplevel"]);
    let folder = top.as_deref().map_or(cwd, Path::new);
    match folder.file_name() {
        Some(name) => name.to_string_lossy().into_owned(),
        None => cwd.display().to_string(),
    }
}
