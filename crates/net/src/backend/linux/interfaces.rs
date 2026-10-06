//! Network interfaces (sidebar rows), their IPv4 state and IPv4 editing.

use super::nm::*;
use crate::model::{Interface, InterfaceKind, Ipv4, Ipv4Config, ManualIpv4};
use std::collections::HashMap;
use std::net::Ipv4Addr;
use zbus::Connection;
use zbus::zvariant::{OwnedObjectPath, OwnedValue, Value};

/// Interfaces in sidebar order, and the label of the one with the default route.
pub async fn list(conn: &Connection) -> zbus::Result<(Vec<Interface>, Option<String>)> {
    let nm = manager(conn).await?;
    let mut found = Vec::new();
    for path in nm.get_devices().await? {
        if let Some(interface) = read_interface(conn, &path).await? {
            found.push(interface);
        }
    }
    found.sort_by_key(|i| kind_rank(i.kind));
    name_interfaces(&mut found);
    let primary = primary_device(conn, &nm)
        .await
        .and_then(|id| found.iter().find(|i| i.id == id))
        .map(|i| i.name.clone());
    Ok((found, primary))
}

fn kind_rank(kind: InterfaceKind) -> u8 {
    match kind {
        InterfaceKind::Wifi => 0,
        InterfaceKind::Ethernet => 1,
        InterfaceKind::Thunderbolt => 2,
        InterfaceKind::Usb => 3,
        InterfaceKind::Other => 4,
    }
}

/// "Ethernet", "Ethernet 2", ... The first of each kind keeps the plain name.
fn name_interfaces(interfaces: &mut [Interface]) {
    let mut counts: HashMap<&'static str, u32> = HashMap::new();
    for interface in interfaces {
        let base = base_name(interface.kind);
        let count = counts.entry(base).or_insert(0);
        *count += 1;
        interface.name = if *count == 1 {
            base.to_string()
        } else {
            format!("{base} {count}")
        };
    }
}

fn base_name(kind: InterfaceKind) -> &'static str {
    match kind {
        InterfaceKind::Wifi => "Wi-Fi",
        InterfaceKind::Ethernet => "Ethernet",
        InterfaceKind::Thunderbolt => "Thunderbolt",
        InterfaceKind::Usb => "USB",
        InterfaceKind::Other => "Network",
    }
}

/// Device path of the active connection that owns the default route.
async fn primary_device(conn: &Connection, nm: &NetworkManagerProxy<'_>) -> Option<String> {
    for path in nm.active_connections().await.ok()? {
        let active = open!(ConnectionActiveProxy, conn, &path).ok()?;
        if active.is_default().await.unwrap_or(false) {
            let devices = active.devices().await.ok()?;
            return devices.first().map(|d| d.to_string());
        }
    }
    None
}

/// None for devices that don't belong in the list (loopback, tun, ...).
async fn read_interface(
    conn: &Connection,
    path: &OwnedObjectPath,
) -> zbus::Result<Option<Interface>> {
    let device = open!(DeviceProxy, conn, path)?;
    if !device.managed().await? {
        return Ok(None);
    }
    let name = device.interface().await?;
    let driver = device.driver().await.unwrap_or_default();
    let Some(kind) = classify(device.device_type().await?, &driver, &name) else {
        return Ok(None);
    };
    let connected = device.state().await? == STATE_ACTIVATED;
    let link_mbps = match (kind, connected) {
        (InterfaceKind::Ethernet | InterfaceKind::Usb, true) => wired_speed(conn, path).await,
        _ => None,
    };
    let summary = summary(conn, path, kind, connected, link_mbps).await;
    let config = read_config(conn, &device, kind).await;
    Ok(Some(Interface {
        id: path.to_string(),
        kind,
        name: base_name(kind).to_string(),
        device: name,
        connected,
        summary,
        ipv4: if connected {
            read_ipv4(conn, &device).await
        } else {
            None
        },
        link_mbps,
        config,
    }))
}

fn classify(device_type: u32, driver: &str, name: &str) -> Option<InterfaceKind> {
    const LOOPBACK: u32 = 32;
    const TUN: u32 = 16;
    const WIFI_P2P: u32 = 30;
    match device_type {
        0 | LOOPBACK | TUN | WIFI_P2P | DEVICE_WIREGUARD => None,
        DEVICE_WIFI => Some(InterfaceKind::Wifi),
        DEVICE_MODEM => Some(InterfaceKind::Usb),
        DEVICE_ETHERNET if is_usb_gadget(driver, name) => Some(InterfaceKind::Usb),
        DEVICE_ETHERNET => Some(InterfaceKind::Ethernet),
        _ => Some(InterfaceKind::Other),
    }
}

/// Phones and gadgets tethered over USB, not USB LAN dongles.
fn is_usb_gadget(driver: &str, name: &str) -> bool {
    driver.starts_with("rndis")
        || driver.starts_with("cdc_")
        || driver == "ipheth"
        || name.starts_with("usb")
}

async fn wired_speed(conn: &Connection, path: &OwnedObjectPath) -> Option<u32> {
    let wired = open!(DeviceWiredProxy, conn, path).ok()?;
    wired.speed().await.ok().filter(|s| *s > 0)
}

async fn summary(
    conn: &Connection,
    path: &OwnedObjectPath,
    kind: InterfaceKind,
    connected: bool,
    link_mbps: Option<u32>,
) -> String {
    if !connected {
        return "not connected".to_string();
    }
    match (kind, link_mbps) {
        (InterfaceKind::Wifi, _) => super::wifi::connected_ssid(conn, path)
            .await
            .unwrap_or_else(|| "connected".to_string()),
        (_, Some(mbps)) => format_speed(mbps),
        _ => "connected".to_string(),
    }
}

fn format_speed(mbps: u32) -> String {
    match mbps {
        m if m >= 1000 && m % 1000 == 0 => format!("{} Gbps", m / 1000),
        m if m >= 1000 => format!("{:.1} Gbps", f64::from(m) / 1000.0),
        m => format!("{m} Mbps"),
    }
}

async fn read_ipv4(conn: &Connection, device: &DeviceProxy<'_>) -> Option<Ipv4> {
    let path = device.ip4_config().await.ok().filter(|p| !is_none(p))?;
    let config = open!(Ip4ConfigProxy, conn, &path).ok()?;
    let addresses = config.address_data().await.ok()?;
    let first = addresses.first()?;
    let router = config.gateway().await.ok().filter(|g| !g.is_empty());
    let dns = config
        .nameserver_data()
        .await
        .unwrap_or_default()
        .iter()
        .filter_map(|d| string_of(d, "address"))
        .collect();
    Some(Ipv4 {
        address: string_of(first, "address")?,
        prefix: u32_of(first, "prefix").and_then(|p| u8::try_from(p).ok())?,
        router,
        dns,
    })
}

/// IPv4 settings of the active connection, else of the first saved one.
/// Wi-Fi only ever shows the active one: saved networks differ per SSID.
async fn read_config(
    conn: &Connection,
    device: &DeviceProxy<'_>,
    kind: InterfaceKind,
) -> Option<Ipv4Config> {
    let path = connection_for(conn, device, kind == InterfaceKind::Wifi).await?;
    let settings = open!(SettingsConnectionProxy, conn, &path)
        .ok()?
        .get_settings()
        .await
        .ok()?;
    Some(config_from_settings(&settings))
}

async fn connection_for(
    conn: &Connection,
    device: &DeviceProxy<'_>,
    active_only: bool,
) -> Option<OwnedObjectPath> {
    if let Ok(active) = device.active_connection().await
        && !is_none(&active)
    {
        let active = open!(ConnectionActiveProxy, conn, &active).ok()?;
        return active.settings_connection().await.ok();
    }
    if active_only {
        return None;
    }
    device
        .available_connections()
        .await
        .ok()?
        .into_iter()
        .next()
}

pub fn config_from_settings(settings: &Settings) -> Ipv4Config {
    let empty = HashMap::new();
    let ipv4 = settings.get("ipv4").unwrap_or(&empty);
    let method = ipv4
        .get("method")
        .and_then(|m| m.downcast_ref::<&str>().ok());
    let addresses = ipv4
        .get("address-data")
        .and_then(|v| Vec::<Dict>::try_from(v.try_clone().ok()?).ok())
        .unwrap_or_default();
    let dns = ipv4
        .get("dns-data")
        .and_then(|v| Vec::<String>::try_from(v.try_clone().ok()?).ok())
        .unwrap_or_default();
    let manual = match (method, addresses.first()) {
        (Some("manual"), Some(first)) => Some(ManualIpv4 {
            address: string_of(first, "address").unwrap_or_default(),
            subnet: prefix_to_mask(u32_of(first, "prefix").unwrap_or(24)),
            router: ipv4
                .get("gateway")
                .and_then(|g| g.downcast_ref::<&str>().ok())
                .unwrap_or_default()
                .to_string(),
        }),
        _ => None,
    };
    Ipv4Config { manual, dns }
}

fn prefix_to_mask(prefix: u32) -> String {
    let bits = u32::MAX.checked_shl(32 - prefix.min(32)).unwrap_or(0);
    Ipv4Addr::from(bits).to_string()
}

fn mask_to_prefix(mask: &str) -> Result<u32, String> {
    let bits = u32::from(
        mask.parse::<Ipv4Addr>()
            .map_err(|_| format!("{mask} is not a valid subnet mask."))?,
    );
    if bits.leading_ones() + bits.trailing_zeros() != 32 {
        return Err(format!("{mask} is not a valid subnet mask."));
    }
    Ok(bits.leading_ones())
}

fn owned(value: Value<'_>) -> Result<OwnedValue, String> {
    OwnedValue::try_from(value).map_err(|e| e.to_string())
}

/// Edit the `ipv4` section of a connection profile in place.
pub fn apply_ipv4(settings: &mut Settings, config: &Ipv4Config) -> Result<(), String> {
    let ipv4 = settings.entry("ipv4".to_string()).or_default();
    for key in [
        "addresses",
        "address-data",
        "gateway",
        "dns",
        "dns-data",
        "ignore-auto-dns",
    ] {
        ipv4.remove(key);
    }
    let method = if config.manual.is_some() {
        "manual"
    } else {
        "auto"
    };
    ipv4.insert("method".to_string(), owned(Value::from(method))?);
    if let Some(manual) = &config.manual {
        let entry: HashMap<&str, Value> = HashMap::from([
            ("address", Value::from(manual.address.as_str())),
            ("prefix", Value::from(mask_to_prefix(&manual.subnet)?)),
        ]);
        ipv4.insert("address-data".to_string(), owned(Value::new(vec![entry]))?);
        if !manual.router.is_empty() {
            ipv4.insert(
                "gateway".to_string(),
                owned(Value::from(manual.router.as_str()))?,
            );
        }
    }
    if !config.dns.is_empty() {
        ipv4.insert(
            "dns-data".to_string(),
            owned(Value::new(config.dns.clone()))?,
        );
        ipv4.insert("ignore-auto-dns".to_string(), owned(Value::from(true))?);
    }
    Ok(())
}

/// Save IPv4 settings to disk and apply them to the running connection.
pub async fn set_ipv4(
    conn: &Connection,
    interface: &str,
    config: &Ipv4Config,
) -> Result<String, String> {
    let what = "save the network settings";
    let path = OwnedObjectPath::try_from(interface).map_err(|e| e.to_string())?;
    let device = open!(DeviceProxy, conn, &path).map_err(|e| explain(what, &e))?;
    let profile = connection_for(conn, &device, false)
        .await
        .ok_or("This interface has no saved connection yet. Connect it first.")?;
    let profile_proxy =
        open!(SettingsConnectionProxy, conn, &profile).map_err(|e| explain(what, &e))?;
    let mut settings = profile_proxy
        .get_settings()
        .await
        .map_err(|e| explain(what, &e))?;
    apply_ipv4(&mut settings, config)?;
    const TO_DISK: u32 = 1;
    profile_proxy
        .update2(settings, TO_DISK, HashMap::new())
        .await
        .map_err(|e| explain(what, &e))?;
    if is_active_on(conn, &device, &profile).await {
        device
            .reapply(HashMap::new(), 0, 0)
            .await
            .map_err(|e| explain("apply the new settings", &e))?;
    }
    Ok("Settings saved.".to_string())
}

async fn is_active_on(
    conn: &Connection,
    device: &DeviceProxy<'_>,
    profile: &OwnedObjectPath,
) -> bool {
    let Ok(active) = device.active_connection().await else {
        return false;
    };
    if is_none(&active) {
        return false;
    }
    let Ok(active) = open!(ConnectionActiveProxy, conn, &active) else {
        return false;
    };
    active.settings_connection().await.ok().as_ref() == Some(profile)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn manual() -> Ipv4Config {
        Ipv4Config {
            manual: Some(ManualIpv4 {
                address: "192.168.1.50".to_string(),
                subnet: "255.255.255.0".to_string(),
                router: "192.168.1.1".to_string(),
            }),
            dns: vec!["1.1.1.1".to_string(), "9.9.9.9".to_string()],
        }
    }

    #[test]
    fn masks_round_trip() {
        assert_eq!(mask_to_prefix("255.255.255.0"), Ok(24));
        assert_eq!(mask_to_prefix("255.255.240.0"), Ok(20));
        assert_eq!(mask_to_prefix("0.0.0.0"), Ok(0));
        assert!(mask_to_prefix("255.0.255.0").is_err());
        assert_eq!(prefix_to_mask(24), "255.255.255.0");
        assert_eq!(prefix_to_mask(0), "0.0.0.0");
    }

    #[test]
    fn manual_settings_round_trip() {
        let mut settings = Settings::new();
        apply_ipv4(&mut settings, &manual()).unwrap();
        assert_eq!(config_from_settings(&settings), manual());
        let ipv4 = &settings["ipv4"];
        assert_eq!(ipv4["method"].downcast_ref::<&str>().unwrap(), "manual");
        assert!(ipv4["ignore-auto-dns"].downcast_ref::<bool>().unwrap());
    }

    #[test]
    fn auto_clears_manual_keys() {
        let mut settings = Settings::new();
        apply_ipv4(&mut settings, &manual()).unwrap();
        let auto = Ipv4Config {
            manual: None,
            dns: vec![],
        };
        apply_ipv4(&mut settings, &auto).unwrap();
        let ipv4 = &settings["ipv4"];
        assert_eq!(ipv4["method"].downcast_ref::<&str>().unwrap(), "auto");
        for key in ["address-data", "gateway", "dns-data", "ignore-auto-dns"] {
            assert!(!ipv4.contains_key(key), "{key} should be gone");
        }
        assert_eq!(config_from_settings(&settings), auto);
    }

    #[test]
    fn keeps_unrelated_settings() {
        let mut settings = Settings::new();
        let ipv4 = settings.entry("ipv4".to_string()).or_default();
        ipv4.insert(
            "never-default".to_string(),
            owned(Value::from(true)).unwrap(),
        );
        apply_ipv4(&mut settings, &manual()).unwrap();
        assert!(settings["ipv4"].contains_key("never-default"));
    }

    #[test]
    fn rejects_bad_mask() {
        let mut config = manual();
        if let Some(m) = config.manual.as_mut() {
            m.subnet = "255.0.255.0".to_string();
        }
        assert!(apply_ipv4(&mut Settings::new(), &config).is_err());
    }

    #[test]
    fn speed_labels() {
        assert_eq!(format_speed(1000), "1 Gbps");
        assert_eq!(format_speed(2500), "2.5 Gbps");
        assert_eq!(format_speed(100), "100 Mbps");
    }

    #[test]
    fn classifies_devices() {
        assert_eq!(
            classify(1, "r8169", "enp34s0"),
            Some(InterfaceKind::Ethernet)
        );
        assert_eq!(
            classify(1, "rndis_host", "enp0s20u1"),
            Some(InterfaceKind::Usb)
        );
        assert_eq!(classify(1, "cdc_ether", "usb0"), Some(InterfaceKind::Usb));
        assert_eq!(classify(2, "iwlwifi", "wlo1"), Some(InterfaceKind::Wifi));
        assert_eq!(classify(32, "", "lo"), None);
        assert_eq!(classify(29, "wireguard", "wg0"), None);
        assert_eq!(classify(16, "tun", "tailscale0"), None);
    }
}
