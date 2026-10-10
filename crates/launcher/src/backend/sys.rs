//! Running other programs: telmo's own binaries, `open`, detached apps.

use super::plan::{Plan, plan_for};
use crate::commands::Action;
use serde::Serialize;
use std::os::unix::process::CommandExt;
use std::path::PathBuf;
use std::process::{Command, Stdio};

/// `telmo-<name>` or `telmo`: next to this executable first, then on PATH.
pub fn find_binary(name: &str) -> Option<PathBuf> {
    let beside = std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(PathBuf::from));
    let path_dirs = std::env::var_os("PATH")
        .map(|p| std::env::split_paths(&p).collect::<Vec<_>>())
        .unwrap_or_default();
    beside
        .into_iter()
        .chain(path_dirs)
        .map(|dir| dir.join(name))
        .find(|p| p.is_file())
}

pub fn clock_installed() -> bool {
    find_binary("telmo-clock").is_some()
}

/// Runs a plan to completion. The failure is the program's own first line of
/// complaint, or a sentence saying it isn't there.
pub fn run_plan(plan: &Plan) -> Result<(), String> {
    let program = find_binary(plan.program)
        .or_else(|| {
            (plan.program == "open" || plan.program == "xdg-open")
                .then(|| PathBuf::from(plan.program))
        })
        .ok_or_else(|| plan.missing.to_string())?;
    let output = Command::new(&program)
        .args(&plan.args)
        .process_group(0)
        .stdin(Stdio::null())
        .output()
        .map_err(|e| format!("{} did not start ({e}).", plan.program))?;
    if output.status.success() {
        return Ok(());
    }
    let stderr = String::from_utf8_lossy(&output.stderr);
    let reason = stderr
        .lines()
        .find(|l| !l.trim().is_empty())
        .unwrap_or("")
        .trim();
    let reason = reason.strip_prefix("telmo-clock: ").unwrap_or(reason);
    if reason.is_empty() {
        Err(format!("{} failed.", plan.program))
    } else {
        Err(reason.to_string())
    }
}

#[derive(Serialize)]
struct Intent<'a> {
    view: &'a str,
    at: u64,
}

/// Tells the System popup, which opens next, what to start with.
fn leave_intent(view: &str) {
    let at = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    let _ = telmo_kit::state::save("system-intent", &Intent { view, at });
}

/// What `Source::run` does for the real systems.
pub fn run_action(action: &Action) -> Result<Option<String>, String> {
    if let Action::SystemView(view) = action {
        leave_intent(view);
    }
    run_plan(&plan_for(action)).map(|()| None)
}

#[cfg_attr(target_os = "macos", allow(dead_code))]
/// Starts a program that outlives us, in a session of its own.
pub fn spawn_detached(program: &str, args: &[String]) -> Result<(), String> {
    let mut command = Command::new(program);
    command
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    // SAFETY: setsid is async-signal-safe and touches no memory of ours.
    unsafe {
        command.pre_exec(|| {
            libc::setsid();
            Ok(())
        });
    }
    let mut child = command
        .spawn()
        .map_err(|e| format!("Couldn't start {program}: {e}."))?;
    // Reap it if it ends while we are still here.
    std::thread::spawn(move || {
        let _ = child.wait();
    });
    Ok(())
}
