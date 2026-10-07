//! Keep awake: a detached `caffeinate` (macOS) or `systemd-inhibit` (Linux)
//! whose pid and end time live in `$XDG_STATE_HOME/telmo/keep-awake.json`, so
//! it keeps running and is found again after the popup closes.

use crate::model::{AwakeChoice, KeepAwake};
use serde::{Deserialize, Serialize};
use std::{
    path::PathBuf,
    process::Stdio,
    time::{SystemTime, UNIX_EPOCH},
};
use tokio::process::Command;

#[derive(Debug, Serialize, Deserialize)]
struct Saved {
    pid: i32,
    /// Unix seconds; None for "until turned off".
    ends: Option<u64>,
}

fn path() -> Option<PathBuf> {
    let base = std::env::var_os("XDG_STATE_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/state")))?;
    Some(base.join("telmo").join("keep-awake.json"))
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

fn load() -> Option<Saved> {
    serde_json::from_slice(&std::fs::read(path()?).ok()?).ok()
}

fn forget() {
    if let Some(path) = path() {
        let _ = std::fs::remove_file(path);
    }
}

#[cfg(target_os = "macos")]
const PROGRAM: &str = "caffeinate";
#[cfg(target_os = "linux")]
const PROGRAM: &str = "systemd-inhibit";

fn command(secs: Option<u64>) -> Command {
    #[cfg(target_os = "macos")]
    let command = {
        let mut command = Command::new("caffeinate");
        command.arg("-dimsu");
        if let Some(secs) = secs {
            command.arg("-t").arg(secs.to_string());
        }
        command
    };
    #[cfg(target_os = "linux")]
    let command = {
        let mut command = Command::new("systemd-inhibit");
        command.args(["--what=idle:sleep", "--why=telmo", "sleep"]);
        command.arg(secs.map_or("infinity".to_string(), |s| s.to_string()));
        command
    };
    command
}

/// Is `pid` still our inhibitor, and not some process that reused the number?
async fn is_ours(pid: i32) -> bool {
    let output = Command::new("ps")
        .args(["-p", &pid.to_string(), "-o", "comm="])
        .stderr(Stdio::null())
        .output()
        .await;
    let Ok(output) = output else { return false };
    String::from_utf8_lossy(&output.stdout).contains(PROGRAM)
}

pub async fn current() -> KeepAwake {
    let Some(saved) = load() else {
        return KeepAwake::Off;
    };
    let expired = saved.ends.is_some_and(|ends| ends <= now());
    if expired || !is_ours(saved.pid).await {
        forget();
        return KeepAwake::Off;
    }
    match saved.ends {
        Some(ends) => KeepAwake::Timed {
            minutes_left: (ends - now()).div_ceil(60) as u32,
        },
        None => KeepAwake::Indefinite,
    }
}

pub async fn set(choice: AwakeChoice) -> Result<String, String> {
    stop().await;
    let secs = match choice {
        AwakeChoice::Off => return Ok("Keep awake is off.".into()),
        AwakeChoice::Minutes(minutes) => Some(u64::from(minutes) * 60),
        AwakeChoice::Indefinite => None,
    };
    start(secs)?;
    Ok(match choice {
        AwakeChoice::Indefinite => "Staying awake until you turn it off.".into(),
        choice => format!("Staying awake for {}.", choice.label()),
    })
}

fn start(secs: Option<u64>) -> Result<(), String> {
    let path = path().ok_or("Could not find a place to remember this. Is HOME set?")?;
    let child = command(secs)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        // Its own process group: closing the popup's terminal must not stop it.
        .process_group(0)
        .spawn()
        .map_err(|e| format!("Could not start {PROGRAM}: {e}. Is it installed?"))?;
    let pid = child
        .id()
        .ok_or("The keep-awake process stopped right away.")? as i32;
    let saved = Saved {
        pid,
        ends: secs.map(|s| now() + s),
    };
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let bytes = serde_json::to_vec(&saved).map_err(|e| format!("Could not save state: {e}"))?;
    std::fs::write(&path, bytes).map_err(|e| {
        kill(pid);
        format!("Could not save state to {}: {e}", path.display())
    })
}

async fn stop() {
    if let Some(saved) = load() {
        if is_ours(saved.pid).await {
            kill(saved.pid);
        }
        forget();
    }
}

/// Stops the whole process group, so `systemd-inhibit`'s `sleep` goes too.
fn kill(pid: i32) {
    // SAFETY: plain signal to a process group we created.
    unsafe {
        if libc::kill(-pid, libc::SIGTERM) != 0 {
            libc::kill(pid, libc::SIGTERM);
        }
    }
}
