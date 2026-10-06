//! VPN rows: Tailscale, NetworkManager VPN/WireGuard profiles, and
//! WireGuard interfaces that something else (wg-quick) manages.

use super::nm::*;
use crate::model::{Vpn, VpnKind};
use serde::Deserialize;
use std::process::Output;
use std::time::Duration;
use tokio::process::Command;
use tokio::time::timeout;
use zbus::Connection;
use zbus::zvariant::OwnedObjectPath;

pub const TAILSCALE_ID: &str = "tailscale";
const EXTERNAL_PREFIX: &str = "external:";
const OPERATOR_HINT: &str = "Run `sudo tailscale set --operator=$USER` once to control Tailscale.";

#[derive(Deserialize)]
struct TailscaleStatus {
    #[serde(rename = "BackendState")]
    backend_state: String,
    #[serde(rename = "TailscaleIPs")]
    ips: Option<Vec<String>>,
}

/// Current Tailscale state, or None when Tailscale isn't installed.
pub async fn tailscale() -> Option<Vpn> {
    let output = run_tailscale(&["status", "--json"], Duration::from_secs(5)).await;
    match output {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
        Err(_) => Some(tailscale_row(
            false,
            Some("Tailscale isn't responding".to_string()),
        )),
        Ok(out) => Some(parse_tailscale(&String::from_utf8_lossy(&out.stdout))),
    }
}

fn parse_tailscale(json: &str) -> Vpn {
    match serde_json::from_str::<TailscaleStatus>(json) {
        Ok(status) => tailscale_row(
            status.backend_state == "Running",
            status.ips.unwrap_or_default().into_iter().next(),
        ),
        Err(_) => tailscale_row(false, Some("Tailscale isn't running".to_string())),
    }
}

fn tailscale_row(connected: bool, detail: Option<String>) -> Vpn {
    Vpn {
        id: TAILSCALE_ID.to_string(),
        name: "Tailscale".to_string(),
        kind: VpnKind::Tailscale,
        connected,
        controllable: true,
        detail,
    }
}

async fn run_tailscale(args: &[&str], limit: Duration) -> std::io::Result<Output> {
    let run = Command::new("tailscale")
        .args(args)
        .kill_on_drop(true)
        .output();
    timeout(limit, run)
        .await
        .unwrap_or_else(|_| Err(std::io::ErrorKind::TimedOut.into()))
}

pub async fn set_tailscale(on: bool) -> Result<String, String> {
    let args: &[&str] = if on {
        &["up", "--timeout", "15s"]
    } else {
        &["down"]
    };
    let out = run_tailscale(args, Duration::from_secs(20))
        .await
        .map_err(|e| format!("Couldn't run tailscale: {e}"))?;
    if out.status.success() {
        let state = if on { "connected" } else { "disconnected" };
        return Ok(format!("Tailscale {state}."));
    }
    Err(tailscale_error(&String::from_utf8_lossy(&out.stderr)))
}

fn tailscale_error(stderr: &str) -> String {
    let lower = stderr.to_lowercase();
    if lower.contains("access denied")
        || lower.contains("permission denied")
        || lower.contains("--operator")
    {
        OPERATOR_HINT.to_string()
    } else if lower.contains("login") || lower.contains("authurl") || lower.contains("https://") {
        "Tailscale needs you to log in. Run `tailscale up` once in a terminal.".to_string()
    } else {
        let line = stderr
            .lines()
            .find(|l| !l.trim().is_empty())
            .unwrap_or("unknown error");
        format!("Tailscale failed: {}", line.trim())
    }
}

/// A VPN-like connection NM currently has active.
struct ActiveVpn {
    uuid: String,
    path: OwnedObjectPath,
    kind: VpnKind,
    external: bool,
    interface: Option<String>,
}

async fn active_vpns(conn: &Connection) -> zbus::Result<Vec<ActiveVpn>> {
    let nm = manager(conn).await?;
    let mut found = Vec::new();
    for path in nm.active_connections().await? {
        let active = open!(ConnectionActiveProxy, conn, &path)?;
        let Some(kind) = vpn_kind(&active.connection_type().await?) else {
            continue;
        };
        let interface = match active.devices().await?.first() {
            Some(device) => open!(DeviceProxy, conn, device)?.interface().await.ok(),
            None => None,
        };
        found.push(ActiveVpn {
            uuid: active.uuid().await?,
            path,
            kind,
            external: active.state_flags().await? & ACTIVE_FLAG_EXTERNAL != 0,
            interface,
        });
    }
    Ok(found)
}

fn vpn_kind(connection_type: &str) -> Option<VpnKind> {
    match connection_type {
        "wireguard" => Some(VpnKind::WireGuard),
        "vpn" => Some(VpnKind::System),
        _ => None,
    }
}

/// Saved profiles and externally managed WireGuard interfaces.
pub async fn network_manager(conn: &Connection) -> zbus::Result<Vec<Vpn>> {
    let active = active_vpns(conn).await?;
    let mut vpns = Vec::new();
    for (_, profile) in saved_profiles(conn).await? {
        vpns.push(Vpn {
            connected: active.iter().any(|a| a.uuid == profile.uuid && !a.external),
            id: profile.uuid,
            name: profile.name,
            kind: profile.kind,
            controllable: true,
            detail: None,
        });
    }
    for a in active.iter().filter(|a| a.external) {
        if let Some(interface) = &a.interface {
            vpns.push(external_row(interface, a.kind, true));
        }
    }
    for interface in unmanaged_wireguard(conn).await? {
        if !vpns.iter().any(|v| v.id == external_id(&interface)) {
            vpns.push(external_row(&interface, VpnKind::WireGuard, true));
        }
    }
    Ok(vpns)
}

fn external_id(interface: &str) -> String {
    format!("{EXTERNAL_PREFIX}{interface}")
}

fn external_row(interface: &str, kind: VpnKind, connected: bool) -> Vpn {
    let detail = match kind {
        VpnKind::WireGuard => format!("Managed by wg-quick-{interface}.service"),
        _ => "Managed outside NetworkManager".to_string(),
    };
    Vpn {
        id: external_id(interface),
        name: interface.to_string(),
        kind,
        connected,
        controllable: false,
        detail: Some(detail),
    }
}

async fn unmanaged_wireguard(conn: &Connection) -> zbus::Result<Vec<String>> {
    let nm = manager(conn).await?;
    let mut names = Vec::new();
    for path in nm.get_devices().await? {
        let device = open!(DeviceProxy, conn, &path)?;
        if device.device_type().await? == DEVICE_WIREGUARD && !device.managed().await? {
            names.push(device.interface().await?);
        }
    }
    Ok(names)
}

struct Profile {
    uuid: String,
    name: String,
    kind: VpnKind,
}

/// Saved (not runtime-only) VPN and WireGuard profiles.
async fn saved_profiles(conn: &Connection) -> zbus::Result<Vec<(OwnedObjectPath, Profile)>> {
    let settings = settings(conn).await?;
    let mut profiles = Vec::new();
    for path in settings.list_connections().await? {
        let proxy = open!(SettingsConnectionProxy, conn, &path)?;
        // Profiles NM invented for externally managed devices are unsaved.
        if proxy.unsaved().await.unwrap_or(false) {
            continue;
        }
        let Ok(dict) = proxy.get_settings().await else {
            continue;
        };
        if let Some(profile) = profile_of(&dict) {
            profiles.push((path, profile));
        }
    }
    Ok(profiles)
}

fn profile_of(settings: &Settings) -> Option<Profile> {
    let connection = settings.get("connection")?;
    let text = |key: &str| {
        connection
            .get(key)?
            .downcast_ref::<&str>()
            .ok()
            .map(str::to_string)
    };
    Some(Profile {
        uuid: text("uuid")?,
        name: text("id")?,
        kind: vpn_kind(&text("type")?)?,
    })
}

pub async fn set_network_manager(conn: &Connection, id: &str, on: bool) -> Result<String, String> {
    if let Some(interface) = id.strip_prefix(EXTERNAL_PREFIX) {
        return Err(format!(
            "{interface} is managed outside NetworkManager, so it can't be toggled here."
        ));
    }
    let what = if on {
        "connect the VPN"
    } else {
        "disconnect the VPN"
    };
    let fail = |e: zbus::Error| explain(what, &e);
    let nm = manager(conn).await.map_err(fail)?;
    if on {
        let profiles = saved_profiles(conn).await.map_err(fail)?;
        let (path, _) = profiles
            .iter()
            .find(|(_, p)| p.uuid == id)
            .ok_or("That VPN profile no longer exists.")?;
        nm.activate_connection(path, &root(), &root())
            .await
            .map_err(fail)?;
        return Ok("VPN connected.".to_string());
    }
    let active = active_vpns(conn).await.map_err(fail)?;
    let found = active
        .iter()
        .find(|a| a.uuid == id)
        .ok_or("That VPN isn't connected.")?;
    nm.deactivate_connection(&found.path).await.map_err(fail)?;
    Ok("VPN disconnected.".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_running_tailscale() {
        let vpn = parse_tailscale(
            r#"{"BackendState":"Running","TailscaleIPs":["100.75.104.50","fd7a::1"],"Self":{}}"#,
        );
        assert!(vpn.connected);
        assert_eq!(vpn.detail.as_deref(), Some("100.75.104.50"));
    }

    #[test]
    fn parses_stopped_tailscale() {
        let vpn = parse_tailscale(r#"{"BackendState":"Stopped","TailscaleIPs":null}"#);
        assert!(!vpn.connected);
        assert_eq!(vpn.detail, None);
    }

    #[test]
    fn garbage_means_not_running() {
        assert!(!parse_tailscale("failed to connect").connected);
    }

    #[test]
    fn operator_error_has_the_hint() {
        let message = tailscale_error("Access denied: checkprefs access denied");
        assert_eq!(message, OPERATOR_HINT);
    }

    #[test]
    fn external_wireguard_is_read_only() {
        let vpn = external_row("wg0", VpnKind::WireGuard, true);
        assert!(!vpn.controllable);
        assert_eq!(
            vpn.detail.as_deref(),
            Some("Managed by wg-quick-wg0.service")
        );
    }
}
