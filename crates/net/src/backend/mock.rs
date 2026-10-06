//! Fake system for `--mock`, UI work and tests.

use super::{Cmd, Event, Rx, Tx};
use crate::model::*;
use std::time::Duration;
use telmo_speed::{Phase, Update};
use tokio::time::sleep;

/// All the fake data lives here: a MacBook on Wi-Fi with a USB LAN adapter.
pub fn data() -> Snapshot {
    let net = |ssid: Option<&str>, strength, security, saved, connected| Network {
        id: ssid.unwrap_or("hidden").to_string(),
        ssid: ssid.map(str::to_string),
        strength,
        security,
        band: Some(Band::G5),
        saved,
        connected,
    };
    let wired =
        |id: &str, kind, name: &str, device: &str, summary: &str, ip: Option<&str>| Interface {
            id: id.to_string(),
            kind,
            name: name.to_string(),
            device: device.to_string(),
            connected: ip.is_some(),
            summary: summary.to_string(),
            ipv4: ip.map(|address| Ipv4 {
                address: address.to_string(),
                prefix: 24,
                router: Some("192.168.1.1".to_string()),
                dns: vec!["1.1.1.1".to_string(), "1.0.0.1".to_string()],
            }),
            link_mbps: ip.map(|_| 1000),
            config: Some(Ipv4Config {
                manual: None,
                dns: vec![],
            }),
        };
    let mut wifi = wired(
        "en0",
        InterfaceKind::Wifi,
        "Wi-Fi",
        "en0",
        "HomeNet-5G",
        Some("192.168.1.42"),
    );
    wifi.link_mbps = None;
    wifi.config = None;
    let vpn = |id: &str, name: &str, kind, connected, detail: Option<&str>| Vpn {
        id: id.to_string(),
        name: name.to_string(),
        kind,
        connected,
        controllable: true,
        detail: detail.map(str::to_string),
    };
    Snapshot {
        interfaces: vec![
            wifi,
            wired(
                "en5",
                InterfaceKind::Ethernet,
                "Ethernet",
                "en5",
                "USB LAN · 1 Gbps",
                Some("192.168.1.40"),
            ),
            wired(
                "en6",
                InterfaceKind::Thunderbolt,
                "Thunderbolt",
                "en6",
                "not connected",
                None,
            ),
            wired(
                "en7",
                InterfaceKind::Usb,
                "iPhone USB",
                "en7",
                "not connected",
                None,
            ),
        ],
        wifi: Some(Wifi {
            interface: "en0".to_string(),
            power: true,
            scanning: false,
            networks: vec![
                net(Some("HomeNet-5G"), 92, Security::Wpa3Personal, true, true),
                net(Some("HomeNet"), 78, Security::Personal, true, false),
                net(Some("Tanaka-AP"), 62, Security::Personal, false, false),
                net(Some("Matv-Guest"), 55, Security::Personal, true, false),
                net(
                    Some("DIRECT-7F-HP OfficeJet"),
                    48,
                    Security::Personal,
                    false,
                    false,
                ),
                net(Some("aterm-5c1d2e-g"), 40, Security::Personal, false, false),
                net(Some("Starbucks_WiFi"), 33, Security::Open, false, false),
                net(None, 22, Security::Personal, false, false),
            ],
            saved_elsewhere: vec!["Office-5G".to_string(), "Parents".to_string()],
            names_hidden: false,
        }),
        vpns: vec![
            vpn(
                "tailscale",
                "Tailscale",
                VpnKind::Tailscale,
                true,
                Some("100.101.12.4"),
            ),
            vpn("work", "Work (IKEv2)", VpnKind::System, false, None),
        ],
        primary: Some("Wi-Fi".to_string()),
        caps: Caps {
            edit_ip: true,
            edit_needs_admin: true,
        },
    }
}

pub fn spawn(mut cmds: Rx, events: Tx) {
    tokio::spawn(async move {
        let mut system = data();
        send(&events, &system);
        while let Some(cmd) = cmds.recv().await {
            handle(&mut system, cmd, &events).await;
        }
    });
}

/// A fake speedtest run, for `--mock` and the same screen as the real one.
pub fn fake_speedtest(tx: Tx) {
    tokio::spawn(async move {
        let send = |update| {
            let _ = tx.send(Event::Speed(update));
        };
        send(Update::Server {
            colo: "AMS".to_string(),
            city: Some("Amsterdam".to_string()),
        });
        sleep(Duration::from_millis(300)).await;
        send(Update::Latency {
            idle_ms: 8.0,
            jitter_ms: 1.2,
        });
        for (phase, target, loaded_ms) in
            [(Phase::Download, 412.6, 24.0), (Phase::Upload, 39.4, 31.0)]
        {
            for step in 1..=20 {
                sleep(Duration::from_millis(120)).await;
                let ramp = (step as f64 / 6.0).min(1.0);
                let wobble = if step % 2 == 0 { 1.04 } else { 0.96 };
                send(Update::Progress {
                    phase,
                    mbps: target * (0.5 + 0.5 * ramp) * wobble,
                    fraction: step as f32 / 20.0,
                });
            }
            send(Update::Result {
                phase,
                mbps: target,
                loaded_ms: Some(loaded_ms),
            });
        }
        send(Update::Finished);
    });
}

fn send(events: &Tx, system: &Snapshot) {
    let _ = events.send(Event::Snapshot(system.clone()));
}

fn done(events: &Tx, target: &str, result: Result<String, String>) {
    let _ = events.send(Event::Done {
        target: target.to_string(),
        result,
    });
}

async fn pause(ms: u64) {
    sleep(Duration::from_millis(ms)).await;
}

async fn handle(system: &mut Snapshot, cmd: Cmd, events: &Tx) {
    match cmd {
        Cmd::Rescan => {
            set_scanning(system, true);
            send(events, system);
            pause(1200).await;
            set_scanning(system, false);
            send(events, system);
            done(events, "wifi", Ok("Scan finished.".to_string()));
        }
        Cmd::Join { network, password } => {
            pause(1500).await;
            if network == "Tanaka-AP" && password.as_deref() != Some("hunter22") {
                let message = "Wrong password for Tanaka-AP.".to_string();
                return done(events, &network, Err(message));
            }
            let name = join(system, &network);
            send(events, system);
            done(events, &network, Ok(format!("Joined {name}.")));
        }
        Cmd::Forget { ssid } => {
            pause(400).await;
            forget(system, &ssid);
            send(events, system);
            done(events, &ssid, Ok(format!("Forgot {ssid}.")));
        }
        Cmd::SetWifiPower(on) => {
            pause(800).await;
            set_power(system, on);
            send(events, system);
            let state = if on { "on" } else { "off" };
            done(events, "wifi", Ok(format!("Wi-Fi is {state}.")));
        }
        Cmd::SetIpv4 { interface, config } => {
            pause(900).await;
            set_ipv4(system, &interface, config);
            send(events, system);
            done(events, &interface, Ok("Settings saved.".to_string()));
        }
        Cmd::SetVpn { vpn, on } => {
            pause(900).await;
            if let Some(v) = system.vpns.iter_mut().find(|v| v.id == vpn) {
                v.connected = on;
            }
            send(events, system);
            let state = if on { "connected" } else { "disconnected" };
            done(events, &vpn, Ok(format!("VPN {state}.")));
        }
        Cmd::Details => {
            pause(500).await;
            let _ = events.send(Event::Details(Details {
                public_ip: Some("203.0.113.24".to_string()),
                channel: Some("36 (80 MHz)".to_string()),
                tx_rate: Some("866 Mbps".to_string()),
            }));
        }
        Cmd::RevealPassword { ssid } => {
            pause(300).await;
            let _ = events.send(Event::Password {
                ssid,
                password: Ok("correct-horse-battery".to_string()),
            });
        }
        Cmd::RequestLocation => {
            pause(600).await;
            if let Some(wifi) = system.wifi.as_mut() {
                wifi.names_hidden = false;
            }
            send(events, system);
            done(events, "wifi", Ok("Location allowed.".to_string()));
        }
    }
}

fn set_scanning(system: &mut Snapshot, scanning: bool) {
    if let Some(wifi) = system.wifi.as_mut() {
        wifi.scanning = scanning;
    }
}

fn wifi_interface(system: &mut Snapshot) -> Option<&mut Interface> {
    let id = system.wifi.as_ref()?.interface.clone();
    system.interfaces.iter_mut().find(|i| i.id == id)
}

/// Connect to a network and return its display name.
fn join(system: &mut Snapshot, id: &str) -> String {
    let mut name = id.to_string();
    if let Some(wifi) = system.wifi.as_mut() {
        for n in wifi.networks.iter_mut() {
            n.connected = n.id == id;
            if n.connected {
                n.saved = true;
                name = n
                    .ssid
                    .clone()
                    .unwrap_or_else(|| "hidden network".to_string());
            }
        }
    }
    if let Some(interface) = wifi_interface(system) {
        interface.connected = true;
        interface.summary = name.clone();
        interface.ipv4 = Some(Ipv4 {
            address: "192.168.1.42".to_string(),
            prefix: 24,
            router: Some("192.168.1.1".to_string()),
            dns: vec!["1.1.1.1".to_string(), "1.0.0.1".to_string()],
        });
    }
    system.primary = Some("Wi-Fi".to_string());
    name
}

fn forget(system: &mut Snapshot, ssid: &str) {
    let mut was_connected = false;
    if let Some(wifi) = system.wifi.as_mut() {
        wifi.saved_elsewhere.retain(|s| s != ssid);
        for n in wifi
            .networks
            .iter_mut()
            .filter(|n| n.ssid.as_deref() == Some(ssid))
        {
            was_connected |= n.connected;
            n.saved = false;
            n.connected = false;
        }
    }
    if was_connected {
        disconnect_wifi(system, "not connected");
    }
}

fn disconnect_wifi(system: &mut Snapshot, summary: &str) {
    if let Some(interface) = wifi_interface(system) {
        interface.connected = false;
        interface.summary = summary.to_string();
        interface.ipv4 = None;
    }
    let ethernet_up = system
        .interfaces
        .iter()
        .any(|i| i.kind == InterfaceKind::Ethernet && i.connected);
    system.primary = ethernet_up.then(|| "Ethernet".to_string());
}

fn set_power(system: &mut Snapshot, on: bool) {
    if let Some(wifi) = system.wifi.as_mut() {
        wifi.power = on;
        for n in wifi.networks.iter_mut() {
            n.connected = false;
        }
    }
    if on {
        join(system, "HomeNet-5G");
    } else {
        disconnect_wifi(system, "off");
    }
}

fn set_ipv4(system: &mut Snapshot, interface: &str, config: Ipv4Config) {
    let Some(i) = system.interfaces.iter_mut().find(|i| i.id == interface) else {
        return;
    };
    if let (Some(manual), Some(ipv4)) = (&config.manual, i.ipv4.as_mut()) {
        ipv4.address = manual.address.clone();
        ipv4.router = Some(manual.router.clone());
    }
    if let Some(ipv4) = i.ipv4.as_mut()
        && !config.dns.is_empty()
    {
        ipv4.dns = config.dns.clone();
    }
    i.config = Some(config);
}
