//! VPNs: Tailscale through its CLI, everything else through `scutil --nc`.

use super::shell;
use crate::model::{Vpn, VpnKind};
use serde_json::Value;
use std::path::PathBuf;
use std::thread;
use std::time::{Duration, Instant};

const TAILSCALE_ID: &str = "tailscale";
const QUICK: Duration = Duration::from_secs(4);
const SCUTIL: &str = "/usr/sbin/scutil";

/// Current state of every VPN we can see.
pub fn poll() -> Vec<Vpn> {
    let mut vpns = Vec::new();
    vpns.extend(tailscale_state());
    vpns.extend(system_vpns());
    vpns
}

/// Connect or disconnect a VPN by its id. Returns the message for the toast.
pub fn set(id: &str, on: bool) -> Result<String, String> {
    let name = if id == TAILSCALE_ID {
        tailscale_set(on)?;
        "Tailscale".to_string()
    } else {
        system_set(id, on)?;
        id.to_string()
    };
    let state = if on { "connected" } else { "disconnected" };
    Ok(format!("{name} {state}."))
}

// Tailscale

fn tailscale_cli() -> Option<PathBuf> {
    let fixed = [
        "/Applications/Tailscale.app/Contents/MacOS/Tailscale",
        "/usr/local/bin/tailscale",
        "/opt/homebrew/bin/tailscale",
    ];
    if let Some(found) = fixed.iter().map(PathBuf::from).find(|p| p.exists()) {
        return Some(found);
    }
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .map(|dir| dir.join("tailscale"))
        .find(|p| p.exists())
}

fn tailscale_state() -> Option<Vpn> {
    let cli = tailscale_cli()?;
    let cli = cli.to_string_lossy();
    let vpn = |connected, detail: Option<String>| Vpn {
        id: TAILSCALE_ID.to_string(),
        name: "Tailscale".to_string(),
        kind: VpnKind::Tailscale,
        connected,
        controllable: true,
        detail,
    };
    let out = match shell::run(&cli, &["status", "--json"], QUICK) {
        Ok(out) => out,
        Err(_) => return Some(vpn(false, Some("Tailscale isn't responding.".to_string()))),
    };
    // `status` exits non-zero when stopped but still prints the state.
    match serde_json::from_str::<Value>(&out.stdout) {
        Ok(json) => {
            let (connected, ip) = parse_tailscale(&json);
            Some(vpn(connected, ip))
        }
        Err(_) => Some(vpn(false, Some("Tailscale isn't running.".to_string()))),
    }
}

fn parse_tailscale(json: &Value) -> (bool, Option<String>) {
    let connected = json.get("BackendState").and_then(Value::as_str) == Some("Running");
    let ips = json
        .get("TailscaleIPs")
        .or_else(|| json.pointer("/Self/TailscaleIPs"))
        .and_then(Value::as_array);
    let ip = ips
        .and_then(|ips| ips.first())
        .and_then(Value::as_str)
        .map(str::to_string);
    (connected, ip)
}

fn tailscale_set(on: bool) -> Result<(), String> {
    let cli = tailscale_cli().ok_or("Tailscale isn't installed.")?;
    let verb = if on { "up" } else { "down" };
    // `up` waits for a login when the node needs one; don't wait forever.
    let out =
        shell::run(&cli.to_string_lossy(), &[verb], Duration::from_secs(20)).map_err(|_| {
            "Tailscale didn't finish. Open the Tailscale app to check on it.".to_string()
        })?;
    if out.ok {
        Ok(())
    } else {
        Err(format!("Tailscale {verb} failed: {}", out.stderr.trim()))
    }
}

// System VPNs (IKEv2, L2TP, Cisco, configuration profiles)

fn system_vpns() -> Vec<Vpn> {
    let Ok(out) = shell::run(SCUTIL, &["--nc", "list"], QUICK) else {
        return vec![];
    };
    out.stdout.lines().filter_map(parse_nc_line).collect()
}

/// `* (Connected)  UUID VPN (com.example) "Work"  [VPN:IKEv2]`
fn parse_nc_line(line: &str) -> Option<Vpn> {
    let state = line.split_once('(')?.1.split_once(')')?.0;
    let name = line.split('"').nth(1)?;
    if line.contains("tailscale") || name == "Tailscale" {
        return None;
    }
    Some(Vpn {
        id: name.to_string(),
        name: name.to_string(),
        kind: VpnKind::System,
        connected: state == "Connected",
        controllable: true,
        detail: None,
    })
}

fn system_set(name: &str, on: bool) -> Result<(), String> {
    let verb = if on { "start" } else { "stop" };
    let out = shell::run(SCUTIL, &["--nc", verb, name], QUICK)?;
    if !out.ok {
        return Err(format!("Couldn't {verb} {name}: {}", out.stderr.trim()));
    }
    let limit = Duration::from_secs(if on { 30 } else { 10 });
    let deadline = Instant::now() + limit;
    while Instant::now() < deadline {
        if system_connected(name) == Some(on) {
            return Ok(());
        }
        thread::sleep(Duration::from_millis(500));
    }
    let verb = if on { "connect" } else { "disconnect" };
    Err(format!(
        "{name} didn't {verb} in time. Check System Settings > VPN."
    ))
}

fn system_connected(name: &str) -> Option<bool> {
    let out = shell::run(SCUTIL, &["--nc", "status", name], QUICK).ok()?;
    Some(out.stdout.lines().next()?.trim() == "Connected")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_nc_list_lines() {
        let line = "* (Connected)      7B3E VPN (com.apple.neagent) \"Work\"  [VPN:IKEv2]";
        let vpn = parse_nc_line(line).expect("work vpn");
        assert_eq!((vpn.name.as_str(), vpn.connected), ("Work", true));
        let ts = "* (Connected)  D2CE VPN (io.tailscale.ipn.macsys) \"Tailscale\"  [VPN:io.tailscale.ipn.macsys]";
        assert!(parse_nc_line(ts).is_none());
        assert!(parse_nc_line("Available network connection services:").is_none());
    }

    #[test]
    fn parses_tailscale_status() {
        let json: Value = serde_json::json!({"BackendState": "Running", "TailscaleIPs": ["100.64.0.1", "fd7a::1"]});
        assert_eq!(
            parse_tailscale(&json),
            (true, Some("100.64.0.1".to_string()))
        );
        let json: Value = serde_json::json!({"BackendState": "Stopped"});
        assert_eq!(parse_tailscale(&json), (false, None));
    }
}
