use crate::fail::{Fail, Result};
use serde_json::Value;

/// The flags that take a value; every other known flag is a switch.
const VALUED: [&str; 6] = ["label", "project", "wait", "agent", "limit", "since"];
const SWITCHES: [&str; 4] = ["json", "check", "help", "version"];

/// The flags given, in the order first given; a repeated one keeps every value, and the last counts.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct Flags(Vec<(String, Vec<String>)>);

impl Flags {
    /// `[("json", ""), ("limit", "2")]` as flags, as the command line would give them.
    pub fn of(pairs: &[(&str, &str)]) -> Flags {
        let mut flags = Flags::default();
        for (key, value) in pairs {
            flags.push(key, value);
        }
        flags
    }

    pub fn push(&mut self, key: &str, value: &str) {
        match self.0.iter_mut().find(|(k, _)| k == key) {
            Some((_, values)) => values.push(value.to_string()),
            None => self.0.push((key.to_string(), vec![value.to_string()])),
        }
    }

    /// The last value given to `--key`.
    pub fn one(&self, key: &str) -> Option<&str> {
        self.0.iter().find(|(k, _)| k == key).and_then(|(_, values)| values.last()).map(String::as_str)
    }

    /// `--key`'s value, None when it is not given or empty.
    pub fn text(&self, key: &str) -> Option<&str> {
        self.one(key).filter(|value| !value.is_empty())
    }

    pub fn has(&self, key: &str) -> bool {
        self.0.iter().any(|(k, _)| k == key)
    }

    /// Refuses any flag `what` does not take.
    pub fn only(&self, what: &str, allowed: &[&str]) -> Result<()> {
        match self.0.iter().find(|(key, _)| !allowed.contains(&key.as_str())) {
            Some((key, _)) => Err(Fail::Usage(format!("{what} takes no --{key}"))),
            None => Ok(()),
        }
    }

    /// `--key` as a non-negative integer, `fallback` when it is not given.
    pub fn integer(&self, key: &str, fallback: u64) -> Result<u64> {
        let Some(raw) = self.text(key) else { return Ok(fallback) };
        let digits = raw.trim();
        let value = if !digits.is_empty() && digits.bytes().all(|b| b.is_ascii_digit()) { digits.parse::<u64>().ok() } else { None };
        value.filter(|v| *v < (1 << 53)).ok_or_else(|| Fail::Usage(format!("--{key} must be a non-negative integer")))
    }
}

#[derive(Debug, Default, PartialEq)]
pub struct Parsed {
    pub positionals: Vec<String>,
    pub flags: Flags,
    /// The words after `--`, the command to run; None with no `--`.
    pub command: Option<Vec<String>>,
}

/// `--key value`, `--key=value`, a switch `--json`, `-h` for `--help` and `-V` for `--version`; repeats accumulate, the last one counts.
pub fn parse(argv: &[String]) -> Result<Parsed> {
    let mut parsed = Parsed::default();
    let mut i = 0;
    while i < argv.len() {
        let token = argv[i].as_str();
        i += 1;
        if token == "--" {
            parsed.command = Some(argv[i..].to_vec());
            return Ok(parsed);
        }
        let short = match token {
            "-h" => Some("help"),
            "-V" => Some("version"),
            _ => None,
        };
        if let Some(key) = short {
            parsed.flags.push(key, "");
            continue;
        }
        if !token.starts_with('-') || token == "-" {
            parsed.positionals.push(token.to_string());
            continue;
        }
        let body = token.get(2..).unwrap_or("");
        let (key, value) = match body.find('=') {
            Some(eq) => (&body[..eq], Some(&body[eq + 1..])),
            None => (body, None),
        };
        let valued = VALUED.contains(&key);
        if !token.starts_with("--") || !(valued || SWITCHES.contains(&key)) {
            return Err(Fail::Usage(format!("unknown option {token}")));
        }
        let value = match value {
            None if valued => match argv.get(i) {
                Some(next) if next != "--" => {
                    i += 1;
                    next.as_str()
                }
                _ => return Err(Fail::Usage(format!("--{key} needs a value"))),
            },
            Some(_) if !valued => return Err(Fail::Usage(format!("--{key} takes no value"))),
            value => value.unwrap_or(""),
        };
        parsed.flags.push(key, value);
    }
    Ok(parsed)
}

/// `payload` as JSON with `--json`, else the human text.
pub fn output(flags: &Flags, payload: Value, human: impl FnOnce() -> String) -> String {
    if flags.has("json") { serde_json::to_string_pretty(&payload).unwrap_or_default() } else { human() }
}

/// A number as JSON gives it: `40`, not `40.0`.
pub fn number(value: f64) -> Value {
    if value.fract() == 0.0 && value.abs() < 1e15 { Value::from(value as i64) } else { Value::from(value) }
}
