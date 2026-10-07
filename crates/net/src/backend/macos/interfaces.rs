//! Network services (Wi-Fi, Ethernet, ...) with their live IPv4 state.

use super::sc::{Store, strings, typed};
use super::shell;
use crate::model::{Interface, InterfaceKind, Ipv4, Ipv4Config, ManualIpv4};
use objc2_core_foundation::CFString;
use objc2_system_configuration::{SCNetworkService, SCNetworkSet, SCPreferences};
use serde_json::Value;
use std::collections::HashMap;
use std::ffi::CString;
use std::net::Ipv4Addr;
use std::time::Duration;

/// One enabled network service from System Settings > Network.
#[derive(Debug, Clone)]
pub struct Service {
    pub id: String,
    /// The name macOS shows, which is what `networksetup` expects.
    pub name: String,
    pub bsd: String,
    pub kind: InterfaceKind,
}

/// What the Wi-Fi side knows, so the Wi-Fi row can show its network name.
pub struct WifiBrief {
    pub power: bool,
    pub ssid: Option<String>,
}

pub struct Built {
    pub interfaces: Vec<Interface>,
    /// Name of the interface carrying the default route.
    pub primary: Option<String>,
}

/// Enabled services we show, in sidebar order.
pub fn services(store: &Store) -> Result<Vec<Service>, String> {
    let prefs = SCPreferences::new(None, &CFString::from_str("telmo-net"), None)
        .ok_or("Couldn't read the network settings.")?;
    let set = SCNetworkSet::current(&prefs).ok_or("Couldn't find the current network set.")?;
    let all = set
        .services()
        .ok_or("Couldn't list the network services.")?;
    let mut found: Vec<Service> = typed::<SCNetworkService>(&all)
        .iter()
        .filter_map(|s| service(&s))
        .filter(|s| !hidden(store, &s.id))
        .collect();
    let order = strings(
        store
            .value("Setup:/Network/Global/IPv4")
            .as_ref()
            .and_then(|v| v.get("ServiceOrder")),
    );
    let position = |s: &Service| {
        order
            .iter()
            .position(|id| *id == s.id)
            .unwrap_or(usize::MAX)
    };
    found.sort_by_key(|s| (rank(s.kind), position(s)));
    Ok(found)
}

/// macOS keeps services for adapters it set up on its own (and that System
/// Settings doesn't list) marked as hidden.
fn hidden(store: &Store, id: &str) -> bool {
    let interface = store.value(&format!("Setup:/Network/Service/{id}/Interface"));
    let flag = interface
        .as_ref()
        .and_then(|i| i.get("HiddenConfiguration"));
    flag.and_then(Value::as_bool).unwrap_or(false)
}

fn rank(kind: InterfaceKind) -> u8 {
    match kind {
        InterfaceKind::Wifi => 0,
        InterfaceKind::Ethernet => 1,
        InterfaceKind::Thunderbolt => 2,
        InterfaceKind::Usb => 3,
        InterfaceKind::Other => 4,
    }
}

fn service(service: &SCNetworkService) -> Option<Service> {
    if !service.enabled() {
        return None;
    }
    let interface = service.interface()?;
    let bsd = interface.bsd_name()?.to_string();
    let kind_name = interface.interface_type()?.to_string();
    let name = service.name()?.to_string();
    let kind = classify(&kind_name, &name)?;
    if kind_name == "Bridge" && !device_exists(&bsd) {
        return None;
    }
    Some(Service {
        id: service.service_id()?.to_string(),
        name,
        bsd,
        kind,
    })
}

/// Map the SystemConfiguration interface type to a kind. VPN-like types are
/// handled elsewhere, so they return None.
fn classify(interface_type: &str, name: &str) -> Option<InterfaceKind> {
    let lower = name.to_lowercase();
    match interface_type {
        "IEEE80211" => Some(InterfaceKind::Wifi),
        "Bridge" if lower.contains("thunderbolt") => Some(InterfaceKind::Thunderbolt),
        "Ethernet" if lower.contains("thunderbolt") => Some(InterfaceKind::Thunderbolt),
        "Ethernet" if lower.contains("iphone") || lower.contains("ipad") => {
            Some(InterfaceKind::Usb)
        }
        "Ethernet" => Some(InterfaceKind::Ethernet),
        "Bridge" | "Bond" | "VLAN" | "FireWire" | "Bluetooth" => Some(InterfaceKind::Other),
        _ => None,
    }
}

fn device_exists(bsd: &str) -> bool {
    let Ok(name) = CString::new(bsd) else {
        return false;
    };
    unsafe { libc::if_nametoindex(name.as_ptr()) != 0 }
}

/// Turn services into sidebar rows using the live state in the store.
pub fn build(
    store: &Store,
    services: &[Service],
    wifi: &WifiBrief,
    speeds: &HashMap<String, u32>,
) -> Built {
    let mut counts: HashMap<&str, u32> = HashMap::new();
    let mut interfaces = Vec::new();
    for service in services.iter().filter(|s| shown(store, s)) {
        let name = display_name(service, &mut counts);
        interfaces.push(interface(store, service, name, wifi, speeds));
    }
    let global = store.value("State:/Network/Global/IPv4");
    let primary_device = global
        .as_ref()
        .and_then(|g| g.get("PrimaryInterface"))
        .and_then(Value::as_str);
    let primary = interfaces
        .iter()
        .find(|i| Some(i.device.as_str()) == primary_device)
        .map(|i| i.name.clone());
    Built {
        interfaces,
        primary,
    }
}

/// Wi-Fi and the odd kinds always show; a wired service shows only while its
/// adapter exists and has a link or an address.
fn shown(store: &Store, service: &Service) -> bool {
    let wired = matches!(
        service.kind,
        InterfaceKind::Ethernet | InterfaceKind::Thunderbolt | InterfaceKind::Usb
    );
    if !wired {
        return true;
    }
    let state = |key: &str| store.value(&format!("State:/Network/Interface/{}/{key}", service.bsd));
    let link = state("Link")
        .and_then(|l| l.get("Active").and_then(Value::as_bool))
        .unwrap_or(false);
    wired_visible(device_exists(&service.bsd), link, state("IPv4").is_some())
}

fn wired_visible(exists: bool, link_active: bool, has_ipv4: bool) -> bool {
    exists && (link_active || has_ipv4)
}

/// "Wi-Fi", "Ethernet", "Ethernet 2", "Thunderbolt", "iPhone USB", ...
fn display_name<'a>(service: &'a Service, counts: &mut HashMap<&'a str, u32>) -> String {
    let base = match service.kind {
        InterfaceKind::Wifi => "Wi-Fi",
        InterfaceKind::Ethernet => "Ethernet",
        InterfaceKind::Thunderbolt => "Thunderbolt",
        InterfaceKind::Usb => return service.name.clone(),
        InterfaceKind::Other => return service.name.clone(),
    };
    let count = counts.entry(base).or_insert(0);
    *count += 1;
    if *count == 1 {
        base.to_string()
    } else {
        format!("{base} {count}")
    }
}

fn interface(
    store: &Store,
    service: &Service,
    name: String,
    wifi: &WifiBrief,
    speeds: &HashMap<String, u32>,
) -> Interface {
    let state = |key: &str| store.value(&format!("State:/Network/Interface/{}/{key}", service.bsd));
    let link_up = state("Link")
        .and_then(|l| l.get("Active").and_then(Value::as_bool))
        .unwrap_or(true);
    let ipv4 = state("IPv4").and_then(|v| ipv4(store, service, &v));
    let is_wifi = service.kind == InterfaceKind::Wifi;
    let wifi_on = !is_wifi || wifi.power;
    let connected = link_up && wifi_on && ipv4.is_some();
    let link_mbps = if !is_wifi && connected {
        speeds.get(&service.bsd).copied()
    } else {
        None
    };
    let summary = if is_wifi && !wifi.power {
        "off".to_string()
    } else if !connected {
        "not connected".to_string()
    } else if is_wifi {
        wifi.ssid.clone().unwrap_or_else(|| "connected".to_string())
    } else {
        link_mbps.map_or_else(|| "connected".to_string(), format_speed)
    };
    Interface {
        id: service.bsd.clone(),
        kind: service.kind,
        name,
        device: service.bsd.clone(),
        connected,
        summary,
        ipv4: connected.then_some(ipv4).flatten(),
        link_mbps,
        config: config(store, service),
    }
}

fn ipv4(store: &Store, service: &Service, state: &Value) -> Option<Ipv4> {
    let address = strings(state.get("Addresses")).into_iter().next()?;
    let mask = strings(state.get("SubnetMasks")).into_iter().next()?;
    let prefix = mask
        .parse::<Ipv4Addr>()
        .map_or(24, |m| m.to_bits().count_ones() as u8);
    let per_service =
        |key: &str| store.value(&format!("State:/Network/Service/{}/{key}", service.id));
    let router = per_service("IPv4")
        .and_then(|v| v.get("Router").and_then(Value::as_str).map(str::to_string));
    let mut dns = per_service("DNS")
        .map(|v| strings(v.get("ServerAddresses")))
        .unwrap_or_default();
    if dns.is_empty() && is_primary(store, service) {
        let global = store.value("State:/Network/Global/DNS");
        dns = strings(global.as_ref().and_then(|g| g.get("ServerAddresses")));
    }
    Some(Ipv4 {
        address,
        prefix,
        router,
        dns,
    })
}

fn is_primary(store: &Store, service: &Service) -> bool {
    let global = store.value("State:/Network/Global/IPv4");
    let primary = global.as_ref().and_then(|g| g.get("PrimaryService"));
    primary.and_then(Value::as_str) == Some(service.id.as_str())
}

/// The settings as edited in the IPv4 dialog (Setup, not the live state).
fn config(store: &Store, service: &Service) -> Option<Ipv4Config> {
    let setup = |key: &str| store.value(&format!("Setup:/Network/Service/{}/{key}", service.id));
    let ipv4 = setup("IPv4")?;
    let manual = (ipv4.get("ConfigMethod").and_then(Value::as_str) == Some("Manual")).then(|| {
        let first = |key: &str| {
            strings(ipv4.get(key))
                .into_iter()
                .next()
                .unwrap_or_default()
        };
        ManualIpv4 {
            address: first("Addresses"),
            subnet: first("SubnetMasks"),
            router: ipv4
                .get("Router")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
        }
    });
    let dns = setup("DNS")
        .map(|v| strings(v.get("ServerAddresses")))
        .unwrap_or_default();
    Some(Ipv4Config { manual, dns })
}

fn format_speed(mbps: u32) -> String {
    if mbps >= 1000 {
        let gbps = mbps as f64 / 1000.0;
        if mbps.is_multiple_of(1000) {
            format!("{} Gbps", mbps / 1000)
        } else {
            format!("{gbps:.1} Gbps")
        }
    } else {
        format!("{mbps} Mbps")
    }
}

/// Negotiated link speed from `ifconfig -v`, e.g. "link rate: 1.0 Gbps".
pub fn link_speed(bsd: &str) -> Option<u32> {
    let out = shell::run("/sbin/ifconfig", &["-v", bsd], Duration::from_secs(3)).ok()?;
    parse_link_rate(&out.stdout)
}

fn parse_link_rate(text: &str) -> Option<u32> {
    let line = text
        .lines()
        .find_map(|l| l.trim().strip_prefix("link rate:"))?;
    let mut words = line.split_whitespace();
    let value: f64 = words.next()?.parse().ok()?;
    let scale = match words.next()? {
        "Gbps" => 1000.0,
        "Mbps" => 1.0,
        "Kbps" => 0.001,
        _ => return None,
    };
    Some((value * scale).round() as u32)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_link_rates() {
        assert_eq!(parse_link_rate("\tlink rate: 1.0 Gbps\n"), Some(1000));
        assert_eq!(parse_link_rate("\tlink rate: 673.92 Mbps\n"), Some(674));
        assert_eq!(parse_link_rate("nothing"), None);
    }

    #[test]
    fn hides_idle_wired() {
        assert!(wired_visible(true, true, false));
        assert!(wired_visible(true, false, true));
        assert!(!wired_visible(true, false, false));
        assert!(!wired_visible(false, true, true));
    }

    #[test]
    fn names_wired_kinds() {
        assert_eq!(
            classify("Ethernet", "AX88179B"),
            Some(InterfaceKind::Ethernet)
        );
        assert_eq!(classify("Ethernet", "iPhone USB"), Some(InterfaceKind::Usb));
        assert_eq!(
            classify("Bridge", "Thunderbolt Bridge"),
            Some(InterfaceKind::Thunderbolt)
        );
        assert_eq!(classify("VPN", "Tailscale"), None);
    }
}
