use crate::fail::{Fail, Result};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// The environment heavy-cmd reads its settings and folders from: the process's, or a test's. An empty
/// variable is unset; on Windows names are case-insensitive, as there.
#[derive(Debug, Default, Clone)]
pub struct Env(HashMap<String, String>);

fn key(name: &str) -> String {
    if cfg!(windows) { name.to_ascii_uppercase() } else { name.to_string() }
}

impl Env {
    pub fn process() -> Env {
        Env(std::env::vars_os().filter_map(|(name, value)| Some((key(name.to_str()?), value.into_string().ok()?))).collect())
    }

    pub fn of(pairs: &[(&str, &str)]) -> Env {
        Env(pairs.iter().map(|(name, value)| (key(name), value.to_string())).collect())
    }

    /// This env with `name` set to `value`; empty unsets it.
    pub fn with(mut self, name: &str, value: &str) -> Env {
        self.0.insert(key(name), value.to_string());
        self
    }

    pub fn get(&self, name: &str) -> Option<&str> {
        self.0.get(&key(name)).map(String::as_str).filter(|value| !value.is_empty())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Config {
    /// The CPU target: the usage, in %, a start waits to be under while others run.
    pub cpu: u32,
    /// How long, in seconds, a command waits to start before it gives up with 75.
    pub wait: u32,
    /// Whether an install updates itself once a day, after a command ends.
    pub auto_update: bool,
}

pub const DEFAULTS: Config = Config { cpu: 60, wait: 900, auto_update: true };

/// Every key the config file takes.
const KEYS: [&str; 3] = ["cpu", "wait", "auto_update"];

fn env_name(key: &str) -> &'static str {
    match key {
        "cpu" => "HEAVY_CMD_CPU",
        "wait" => "HEAVY_CMD_WAIT",
        _ => "HEAVY_CMD_AUTO_UPDATE",
    }
}

fn rule(key: &str) -> &'static str {
    match key {
        "cpu" => "an integer between 10 and 100",
        "wait" => "an integer number of seconds between 0 and 86400",
        _ => "on or off",
    }
}

fn integer_in(raw: &str, min: u64, max: u64) -> Option<u32> {
    if raw.is_empty() || !raw.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    raw.parse::<u64>().ok().filter(|value| (min..=max).contains(value)).map(|value| value as u32)
}

/// Sets `key` from `raw`, else a refusal naming `place` and why.
fn set(config: &mut Config, key: &str, raw: &str, place: &str) -> Result<()> {
    let text = raw.trim();
    let done = match key {
        "cpu" => integer_in(text, 10, 100).map(|value| config.cpu = value),
        "wait" => integer_in(text, 0, 86_400).map(|value| config.wait = value),
        _ => match text {
            "on" => Some(true),
            "off" => Some(false),
            _ => None,
        }
        .map(|value| config.auto_update = value),
    };
    done.ok_or_else(|| Fail::Config(format!("{place} must be {}, not \"{raw}\"", rule(key))))
}

/// The home folder: `HOME` (`USERPROFILE` on Windows), else the system's.
fn home(env: &Env) -> PathBuf {
    let name = if cfg!(windows) { "USERPROFILE" } else { "HOME" };
    env.get(name).map(PathBuf::from).or_else(std::env::home_dir).unwrap_or_default()
}

/// An XDG base folder: the env's when it is an absolute path, as the spec asks; else Windows' own
/// `windows` folder (`LOCALAPPDATA`, `APPDATA`), or `~/<fallback>`.
fn base(env: &Env, name: &str, windows: (&str, &str), fallback: &str) -> PathBuf {
    if let Some(value) = env.get(name).filter(|value| Path::new(value).is_absolute()) {
        return PathBuf::from(value);
    }
    if cfg!(windows) {
        return env.get(windows.0).map(PathBuf::from).unwrap_or_else(|| home(env).join(windows.1));
    }
    home(env).join(fallback)
}

/// The queue and its record: `$HEAVY_CMD_HOME`, else `${XDG_STATE_HOME:-~/.local/state}/heavy-cmd`
/// (`%LOCALAPPDATA%\heavy-cmd` on Windows).
pub fn state_dir(env: &Env) -> PathBuf {
    match env.get("HEAVY_CMD_HOME") {
        Some(dir) => std::path::absolute(dir).unwrap_or_else(|_| PathBuf::from(dir)),
        None => base(env, "XDG_STATE_HOME", ("LOCALAPPDATA", "AppData/Local"), ".local/state").join("heavy-cmd"),
    }
}

/// `${XDG_CONFIG_HOME:-~/.config}/heavy-cmd/config` (`%APPDATA%\heavy-cmd\config` on Windows).
pub fn config_file(env: &Env) -> PathBuf {
    base(env, "XDG_CONFIG_HOME", ("APPDATA", "AppData/Roaming"), ".config").join("heavy-cmd").join("config")
}

/// The config file's `key = value` lines over `config`: blank lines and `#` comments are skipped, the
/// last of a repeated key counts. A file that is not there sets nothing; an unknown key, a line that is
/// not `key = value` or a value out of its bounds is refused, naming the line.
fn read_file(config: &mut Config, file: &Path) -> Result<()> {
    let text = match std::fs::read_to_string(file) {
        Ok(text) => text,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(Fail::Config(format!("{}: cannot read it: {error}", file.display()))),
    };
    for (index, line) in text.split('\n').enumerate() {
        let content = line.split('#').next().unwrap_or("").trim();
        if content.is_empty() {
            continue;
        }
        let place = format!("{}:{}", file.display(), index + 1);
        let name_len = content.bytes().take_while(|b| b.is_ascii_alphabetic() || *b == b'_').count();
        let (name, rest) = content.split_at(name_len);
        let Some(value) = rest.trim_start().strip_prefix('=').filter(|_| name_len > 0) else {
            return Err(Fail::Config(format!("{place}: expected key = value, not \"{content}\"")));
        };
        if !KEYS.contains(&name) {
            return Err(Fail::Config(format!("{place}: unknown key {name}; the keys are {}", KEYS.join(", "))));
        }
        set(config, name, value.trim_start(), &format!("{place}: {name}"))?;
    }
    Ok(())
}

/// The settings in force: the defaults, then the config file, then the env (`HEAVY_CMD_CPU`,
/// `HEAVY_CMD_WAIT`, `HEAVY_CMD_AUTO_UPDATE`; empty is unset), then `wait`, the raw `--wait`.
pub fn load_config(env: &Env, wait: Option<&str>) -> Result<Config> {
    let mut config = DEFAULTS;
    read_file(&mut config, &config_file(env))?;
    for key in KEYS {
        if let Some(raw) = env.get(env_name(key)) {
            set(&mut config, key, raw, env_name(key))?;
        }
    }
    if let Some(raw) = wait {
        set(&mut config, "wait", raw, "--wait")?;
    }
    Ok(config)
}
