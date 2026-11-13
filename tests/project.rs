mod common;

use heavy_cmd::project::repo_name;

#[test]
fn a_remotes_repository_name_whatever_its_form() {
    assert_eq!(repo_name("git@github.com:Ploppy3/heavy-cmd.git").as_deref(), Some("heavy-cmd"));
    assert_eq!(repo_name("https://gitlab.example.com/group/sub/heavy-cmd/").as_deref(), Some("heavy-cmd"));
    assert_eq!(repo_name("ssh://git@host:2222/heavy-cmd").as_deref(), Some("heavy-cmd"));
    assert_eq!(repo_name("/srv/git/heavy-cmd.git").as_deref(), Some("heavy-cmd"));
    assert_eq!(repo_name(r"C:\repos\heavy-cmd.git").as_deref(), Some("heavy-cmd"));
    assert_eq!(repo_name(""), None);
}

/// git is a Linux test's: Wine runs no git.
#[cfg(unix)]
mod git {
    use super::common::TempDir;
    use heavy_cmd::project::project_of;
    use std::path::Path;
    use std::process::Command;

    fn git(cwd: &Path, args: &[&str]) {
        assert!(Command::new("git").arg("-C").arg(cwd).args(args).output().unwrap().status.success(), "git {args:?}");
    }

    #[test]
    fn origins_repository_name_first_a_worktrees_folder_is_not_the_project() {
        let dir = TempDir::new("heavy-cmd-project");
        let repo = dir.join("checkout");
        std::fs::create_dir(&repo).unwrap();
        git(&repo, &["init", "-q"]);
        git(&repo, &["remote", "add", "origin", "git@github.com:acme/api.git"]);
        git(&repo, &["-c", "user.name=t", "-c", "user.email=t@t", "commit", "-q", "--allow-empty", "-m", "init"]);
        let worktree = dir.join("task-73-fix-login");
        git(&repo, &["worktree", "add", "-q", worktree.to_str().unwrap()]);
        std::fs::create_dir(worktree.join("src")).unwrap();
        assert_eq!(project_of(&worktree.join("src")), "api");
        assert_eq!(project_of(&repo), "api");
    }

    #[test]
    fn with_no_origin_the_git_top_level_folders_name_outside_git_the_cwds_name() {
        let dir = TempDir::new("heavy-cmd-project");
        let repo = dir.join("scratch");
        std::fs::create_dir_all(repo.join("deep").join("er")).unwrap();
        git(&repo, &["init", "-q"]);
        assert_eq!(project_of(&repo.join("deep").join("er")), "scratch");
        let plain = dir.join("plain");
        std::fs::create_dir(&plain).unwrap();
        assert_eq!(project_of(&plain), "plain");
    }
}
