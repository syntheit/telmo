//! Hyprland: one row per process that owns windows, memory and CPU from /proc.
#![cfg_attr(not(target_os = "linux"), allow(dead_code))]

use super::Raw;
use serde_json::Value;
use std::{collections::BTreeMap, process::Command};

pub fn list() -> Result<Vec<Raw>, String> {
    let clients = clients()?;
    let ticks = ticks_per_second();
    let mut by_pid: BTreeMap<i32, Raw> = BTreeMap::new();
    for client in &clients {
        let Some(pid) = client["pid"].as_i64().and_then(|p| i32::try_from(p).ok()) else {
            continue;
        };
        let class = client["class"].as_str().unwrap_or("").trim();
        if class.is_empty() || pid <= 0 {
            continue;
        }
        if let Some(raw) = by_pid.get_mut(&pid) {
            raw.windows += 1;
            continue;
        }
        // The process may have just exited; then there is nothing to show.
        let (Some(memory), Some(cpu_ticks)) = (memory(pid), cpu_ticks(pid)) else {
            continue;
        };
        by_pid.insert(
            pid,
            Raw {
                pid,
                name: class.to_string(),
                memory,
                cpu_ns: cpu_ticks * 1_000_000_000 / ticks,
                unresponsive: false,
                windows: 1,
            },
        );
    }
    Ok(by_pid.into_values().collect())
}

fn clients() -> Result<Vec<Value>, String> {
    let output = Command::new("hyprctl")
        .args(["clients", "-j"])
        .output()
        .map_err(|e| format!("hyprctl did not start ({e}). Is Hyprland running?"))?;
    if !output.status.success() {
        return Err("hyprctl could not list the windows. Is Hyprland running?".into());
    }
    let json: Value = serde_json::from_slice(&output.stdout)
        .map_err(|_| "hyprctl gave an answer telmo could not read.".to_string())?;
    json.as_array()
        .cloned()
        .ok_or_else(|| "hyprctl gave an answer telmo could not read.".to_string())
}

fn ticks_per_second() -> u64 {
    // SAFETY: sysconf only reads a constant.
    let ticks = unsafe { libc::sysconf(libc::_SC_CLK_TCK) };
    u64::try_from(ticks).ok().filter(|t| *t > 0).unwrap_or(100)
}

fn memory(pid: i32) -> Option<u64> {
    let status = std::fs::read_to_string(format!("/proc/{pid}/status")).ok()?;
    parse_rss(&status)
}

fn cpu_ticks(pid: i32) -> Option<u64> {
    let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    parse_cpu_ticks(&stat)
}

/// VmRSS in bytes from /proc/<pid>/status.
fn parse_rss(status: &str) -> Option<u64> {
    let line = status.lines().find_map(|l| l.strip_prefix("VmRSS:"))?;
    let kb: u64 = line.split_whitespace().next()?.parse().ok()?;
    Some(kb * 1024)
}

/// utime + stime from /proc/<pid>/stat. The name is in parentheses and may
/// contain spaces, so count fields from the last `)`.
fn parse_cpu_ticks(stat: &str) -> Option<u64> {
    let after_name = &stat[stat.rfind(')')? + 1..];
    let mut fields = after_name.split_whitespace();
    // Fields 14 and 15 overall; the first one after the name is field 3.
    let utime: u64 = fields.nth(11)?.parse().ok()?;
    let stime: u64 = fields.next()?.parse().ok()?;
    Some(utime + stime)
}

/// Asks every window of the app to close.
pub fn quit(pid: i32, name: &str, force: bool) -> Result<(), String> {
    if force {
        return kill(pid, name);
    }
    let addresses: Vec<String> = clients()?
        .iter()
        .filter(|c| c["pid"].as_i64() == Some(i64::from(pid)))
        .filter_map(|c| c["address"].as_str().map(str::to_string))
        .collect();
    if addresses.is_empty() {
        return Err(format!("{name} has no window left to close."));
    }
    for address in addresses {
        let target = format!("address:{address}");
        let status = Command::new("hyprctl")
            .args(["dispatch", "closewindow", &target])
            .output()
            .map_err(|e| format!("hyprctl did not start ({e}). Is Hyprland running?"))?
            .status;
        if !status.success() {
            return Err(format!("Could not close {name}. Press K to force it."));
        }
    }
    Ok(())
}

fn kill(pid: i32, name: &str) -> Result<(), String> {
    // SAFETY: kill takes plain integers and has no memory effects.
    if unsafe { libc::kill(pid, libc::SIGKILL) } == 0 {
        return Ok(());
    }
    let why = std::io::Error::last_os_error();
    match why.raw_os_error() {
        Some(libc::ESRCH) => Ok(()),
        Some(libc::EPERM) => Err(format!("{name} belongs to another user. Use sudo.")),
        _ => Err(format!("Could not force quit {name} ({why}). Try again.")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_rss_in_bytes() {
        let status = "Name:\tzen\nVmPeak:\t 9 kB\nVmRSS:\t  2048 kB\nThreads:\t4\n";
        assert_eq!(parse_rss(status), Some(2048 * 1024));
        assert_eq!(parse_rss("Name:\tkthreadd\n"), None);
    }

    #[test]
    fn reads_cpu_ticks_past_a_name_with_spaces() {
        let stat = "42 (my app (x)) S 1 42 42 0 -1 4194560 100 0 0 0 30 12 0 0 20 0 1 0 5 1000 50";
        assert_eq!(parse_cpu_ticks(stat), Some(42));
    }
}
