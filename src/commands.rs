use crate::cli::{Flags, output};
use crate::config::{Env, load_config, state_dir};
use crate::cpu::usage_line;
use crate::fail::{Fail, Result};
use crate::format::{clock, duration, now_ms, parse_iso, warn};
use crate::project::project_of;
use crate::queue::{ClaimOptions, Heavy, HeavyPorts, HeavyStatus, read_runs};
use crate::run::{AGENT, INSIDE, WAITING, run_heavy};
use crate::runs::{HeavyRun, RAN, RunFilter, since_of};
use crate::sys::Stop;
use serde_json::Value;
use std::path::Path;

/// `2 running · cpu 63%/60%, last start 42s ago`.
fn summary(status: &HeavyStatus, target: u32, now: i64) -> String {
    let last = match status.last_start.as_deref().and_then(parse_iso) {
        Some(at) => format!("last start {} ago", duration(now - at)),
        None => "no start yet".to_string(),
    };
    format!("{}, {last}", usage_line(status.running.len(), status.cpu, target))
}

/// The summary, then the running commands and the waiters, oldest first.
pub fn status_lines(status: &HeavyStatus, target: u32, now: i64) -> Vec<String> {
    let mut lines = vec![summary(status, target, now)];
    for h in &status.running {
        lines.push(format!("running  {}: {} (pid {}, since {})", h.who.project, h.who.label, h.who.pid, h.started_at));
    }
    for (index, w) in status.waiting.iter().enumerate() {
        lines.push(format!("waiting  {}. {}: {} (pid {}, since {})", index + 1, w.who.project, w.who.label, w.who.pid, w.since));
    }
    lines
}

/// `14:02:11  heavy-cmd  npm run check  exit 0  waited 12s  ran 3m 4s  task-73`: when it ended, its date
/// too on another day than `now`'s.
pub fn run_line(run: &HeavyRun, now: i64) -> String {
    let ended = parse_iso(&run.ended_at).unwrap_or(0);
    let started = run.started_at.as_deref().and_then(parse_iso).unwrap_or(ended);
    let waited = duration(started - parse_iso(&run.queued_at).unwrap_or(started));
    let how = if run.outcome == RAN {
        let exit = run.exit.map_or_else(|| "?".to_string(), |exit| exit.to_string());
        format!("exit {exit}  waited {waited}  ran {}", duration(ended - started))
    } else {
        let exit = run.exit.map(|exit| format!(" ({exit})")).unwrap_or_default();
        format!("{} after {waited}{exit}", run.outcome)
    };
    let mut parts = vec![clock(ended, now, true), run.project.clone(), run.label.clone(), how];
    parts.extend(run.agent.clone());
    parts.extend(run.inside.map(|inside| format!("inside heavy {inside}")));
    parts.join("  ")
}

/// `heavy-cmd -- <command>`: runs it once the machine has room, and returns its exit. Its project is
/// `--project`, else the cwd's (`project_of`); its agent `--agent`, else `HEAVY_CMD_AGENT`; its label
/// `--label`, else the command's first 80 characters. `stop` ends its wait.
pub fn cmd_run(command: &[String], flags: &Flags, env: &Env, cwd: &Path, stop: Option<&'static Stop>) -> Result<i32> {
    let config = load_config(env, flags.one("wait"))?;
    let project = flags.text("project").map_or_else(|| project_of(cwd), str::to_string);
    let heavy = Heavy::open(&state_dir(env), HeavyPorts::machine())?;
    let on_wait = move |status: &HeavyStatus| {
        let by: Vec<String> = status.running.iter().map(|h| format!("{}: {}", h.who.project, h.who.label)).collect();
        let by = if by.is_empty() { String::new() } else { format!(" ({})", by.join(", ")) };
        warn(&format!("heavy-cmd: {WAITING}: {}{by}; waiting up to {}s", summary(status, config.cpu, now_ms()), config.wait));
    };
    let mut options = ClaimOptions {
        label: flags.text("label").map_or_else(|| command.join(" ").chars().take(80).collect(), str::to_string),
        project,
        agent: flags.text("agent").or(env.get(AGENT)).map(str::to_string),
        cwd: Some(cwd.display().to_string()),
        inside: env.get(INSIDE).map(str::to_string),
        wait_sec: u64::from(config.wait),
        cpu: config.cpu,
        on_wait: Some(Box::new(on_wait)),
        stop,
        ..ClaimOptions::default()
    };
    run_heavy(&heavy, command, &mut options)
}

/// `heavy-cmd status [--json]`: the running commands, the waiters in order, the CPU against its target
/// and the time since the last start.
pub fn cmd_status(flags: &Flags, env: &Env) -> Result<String> {
    let config = load_config(env, None)?;
    let status = Heavy::open(&state_dir(env), HeavyPorts::machine())?.status()?;
    let now = now_ms();
    let since = status.last_start.as_deref().and_then(parse_iso).map(|at| ((now - at) as f64 / 1000.0).round() as i64);
    let mut payload = status.to_json();
    payload.insert("target".into(), config.cpu.into());
    payload.insert("since_last_start_sec".into(), since.into());
    Ok(output(flags, Value::Object(payload), || status_lines(&status, config.cpu, now).join("\n")))
}

/// `heavy-cmd logs`: the machine's record of heavy commands, or one project's, the last ended first, 50 unless `--limit`.
pub fn cmd_logs(flags: &Flags, env: &Env, now: i64) -> Result<String> {
    let since = flags.text("since").map(|since| since_of(since, now, "--since")).transpose().map_err(Fail::Usage)?;
    let filter = RunFilter { project: flags.text("project").map(str::to_string), limit: flags.integer("limit", 50)?, since };
    let runs = read_runs(&state_dir(env), &filter)?;
    let payload = Value::Array(runs.iter().map(HeavyRun::to_json).collect());
    Ok(output(flags, payload, || {
        if runs.is_empty() {
            "no heavy command recorded".to_string()
        } else {
            runs.iter().map(|run| run_line(run, now)).collect::<Vec<_>>().join("\n")
        }
    }))
}
