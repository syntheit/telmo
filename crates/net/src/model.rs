//! Plain data shared by the UI and every backend. Backends send a whole
//! `Snapshot` whenever anything changes; the UI never patches state itself.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Snapshot {
    /// Interfaces in sidebar order. VPNs are listed separately.
    pub interfaces: Vec<Interface>,
    pub wifi: Option<Wifi>,
    pub vpns: Vec<Vpn>,
    /// Label of the interface carrying the default route, e.g. "Wi-Fi".
    pub primary: Option<String>,
    pub caps: Caps,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Caps {
    /// Can change IPv4/DNS settings (macOS asks for an admin password).
    pub edit_ip: bool,
    /// Saving IP settings shows an admin password prompt.
    pub edit_needs_admin: bool,
    /// Showing a saved Wi-Fi password asks for Touch ID (macOS).
    #[serde(default)]
    pub reveal_touch_id: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum InterfaceKind {
    Wifi,
    Ethernet,
    Thunderbolt,
    Usb,
    Other,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Interface {
    /// Stable id the backend understands (BSD name, NM device path, ...).
    pub id: String,
    pub kind: InterfaceKind,
    /// Human name: "Wi-Fi", "Ethernet", "iPhone USB".
    pub name: String,
    /// Device name: "en0", "enp5s0".
    pub device: String,
    pub connected: bool,
    /// One line under the name in the sidebar: "HomeNet-5G", "1 Gbps", "not connected".
    pub summary: String,
    pub ipv4: Option<Ipv4>,
    pub link_mbps: Option<u32>,
    pub config: Option<Ipv4Config>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Ipv4 {
    pub address: String,
    pub prefix: u8,
    pub router: Option<String>,
    pub dns: Vec<String>,
}

/// How an interface is configured, as edited in the IPv4 dialog.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Ipv4Config {
    pub manual: Option<ManualIpv4>,
    /// Empty means automatic DNS.
    pub dns: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ManualIpv4 {
    pub address: String,
    pub subnet: String,
    pub router: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Wifi {
    /// Interface id this Wi-Fi state belongs to.
    pub interface: String,
    pub power: bool,
    pub scanning: bool,
    /// In range, strongest first. The connected network is included.
    pub networks: Vec<Network>,
    /// Saved networks that are not in range.
    pub saved_elsewhere: Vec<String>,
    /// macOS hides names without Location permission.
    pub names_hidden: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Network {
    /// Backend id used to join (SSID, AP path, ...).
    pub id: String,
    /// None for hidden networks or names hidden by the OS.
    pub ssid: Option<String>,
    /// 0–100.
    pub strength: u8,
    pub security: Security,
    pub band: Option<Band>,
    pub saved: bool,
    pub connected: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Security {
    Open,
    Wep,
    Personal,
    Wpa3Personal,
    Enterprise,
}

impl Security {
    pub fn needs_password(self) -> bool {
        !matches!(self, Security::Open)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Band {
    G2,
    G5,
    G6,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum VpnKind {
    Tailscale,
    WireGuard,
    System,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Vpn {
    pub id: String,
    pub name: String,
    pub kind: VpnKind,
    pub connected: bool,
    /// False when the OS won't let us toggle it (e.g. a wg-quick unit).
    pub controllable: bool,
    /// Extra line for details, e.g. the Tailscale IP or why it's read-only.
    pub detail: Option<String>,
}

/// Extra facts shown in the Wi-Fi details dialog, fetched on demand.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Details {
    pub public_ip: Option<String>,
    pub channel: Option<String>,
    pub tx_rate: Option<String>,
}
