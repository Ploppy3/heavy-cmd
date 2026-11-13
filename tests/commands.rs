mod common;

use common::TempDir;
use heavy_cmd::cli::Flags;
use heavy_cmd::commands::{cmd_run, cmd_status};
use heavy_cmd::config::Env;
use heavy_cmd::queue::read_runs;
use heavy_cmd::runs::{HeavyRun, RunFilter};
use heavy_cmd::serde_json;

/// A temp state and config, and nothing else of the machine's.
fn env_in(dir: &TempDir) -> Env {
    let at = |name: &str| dir.join(name).to_string_lossy().into_owned();
    Env::of(&[("HEAVY_CMD_HOME", &at("state")), ("XDG_CONFIG_HOME", &at("config")), ("HOME", &at("home")), ("USERPROFILE", &at("home"))])
}

fn runs(dir: &TempDir) -> Vec<HeavyRun> {
    read_runs(&dir.join("state"), &RunFilter { limit: 5, ..RunFilter::default() }).unwrap()
}

fn argv(words: &[&str]) -> Vec<String> {
    words.iter().map(|word| word.to_string()).collect()
}

#[test]
fn a_command_is_recorded_under_project_agent_and_label_when_given() {
    let dir = TempDir::new("heavy-cmd-commands");
    let flags = Flags::of(&[("project", "web"), ("agent", "task-9"), ("label", "the suite")]);
    let env = env_in(&dir).with("HEAVY_CMD_AGENT", "other");
    assert_eq!(cmd_run(&argv(&["exit 2"]), &flags, &env, dir.path(), None).unwrap(), 2);
    let run = &runs(&dir)[0];
    assert_eq!((run.project.as_str(), run.agent.as_deref(), run.label.as_str()), ("web", Some("task-9"), "the suite"));
    assert_eq!((run.cwd.clone(), run.exit, run.command.clone()), (dir.path().display().to_string(), Some(2), Some(argv(&["exit 2"]))));
}

#[cfg(unix)]
#[test]
fn else_under_the_cwds_project_heavy_cmd_agent_and_the_commands_first_80_characters() {
    let dir = TempDir::new("heavy-cmd-commands");
    let repo = dir.join("worktree-folder");
    std::fs::create_dir(&repo).unwrap();
    let git = |args: &[&str]| assert!(std::process::Command::new("git").arg("-C").arg(&repo).args(args).status().unwrap().success());
    git(&["init", "-q"]);
    git(&["remote", "add", "origin", "https://github.com/acme/api.git"]);
    let long = format!("true {}", "x".repeat(100));
    assert_eq!(
        cmd_run(std::slice::from_ref(&long), &Flags::default(), &env_in(&dir).with("HEAVY_CMD_AGENT", "task-73"), &repo, None).unwrap(),
        0
    );
    let run = &runs(&dir)[0];
    assert_eq!((run.project.as_str(), run.agent.as_deref(), run.label.as_str()), ("api", Some("task-73"), &long[..80]));
    assert_eq!(run.cwd, repo.display().to_string());
}

#[test]
fn with_no_agent_named_none_is_recorded() {
    let dir = TempDir::new("heavy-cmd-commands");
    cmd_run(&argv(&["exit 0"]), &Flags::default(), &env_in(&dir), dir.path(), None).unwrap();
    assert_eq!(runs(&dir)[0].agent, None);
}

#[test]
fn a_setting_out_of_bounds_is_refused_before_anything_runs() {
    let dir = TempDir::new("heavy-cmd-commands");
    let refused = cmd_run(&argv(&["exit 0"]), &Flags::of(&[("wait", "90000")]), &env_in(&dir), dir.path(), None).unwrap_err();
    assert!(refused.to_string().starts_with("--wait must be an integer number of seconds between 0 and 86400"), "{refused}");
    assert!(runs(&dir).is_empty());
}

#[test]
fn status_json_has_the_fields_of_factory_heavy_status_json_the_target_from_the_config() {
    let dir = TempDir::new("heavy-cmd-commands");
    cmd_run(&argv(&["exit 0"]), &Flags::default(), &env_in(&dir), dir.path(), None).unwrap();
    let text = cmd_status(&Flags::of(&[("json", "")]), &env_in(&dir).with("HEAVY_CMD_CPU", "70")).unwrap();
    let status: serde_json::Value = serde_json::from_str(&text).unwrap();
    let keys: Vec<&str> = status.as_object().unwrap().keys().map(String::as_str).collect();
    assert_eq!(keys, ["running", "waiting", "cpu", "last_start", "target", "since_last_start_sec"]);
    assert_eq!((status["running"].as_array().unwrap().len(), status["waiting"].as_array().unwrap().len()), (0, 0));
    assert_eq!(status["target"], 70);
    assert!(status["since_last_start_sec"].as_i64().unwrap() >= 0);
}

#[test]
fn the_human_status_starts_with_the_summary() {
    let dir = TempDir::new("heavy-cmd-commands");
    assert_eq!(cmd_status(&Flags::default(), &env_in(&dir)).unwrap(), "0 running · cpu ?/60%, no start yet");
}
