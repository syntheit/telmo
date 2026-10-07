//! Linux backend: talks to NetworkManager over the system D-Bus, and to the
//! `tailscale` CLI for Tailscale.

mod interfaces;
mod nm;
mod vpn;
mod wifi;

use super::{Cmd, Event, Rx, Tx};
use crate::model::{Caps, Details, Snapshot};
use futures_util::StreamExt;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;
use tokio::sync::{Mutex, Notify};
use tokio::time::{Instant, interval_at, sleep};
use zbus::message::Type;
use zbus::{Connection, MatchRule, MessageStream};

const DEBOUNCE: Duration = Duration::from_millis(30);
const TAILSCALE_POLL: Duration = Duration::from_secs(5);

pub fn spawn(cmds: Rx, events: Tx) {
    tokio::spawn(run(cmds, events));
}

/// State shared by the command tasks and the watcher.
struct Shared {
    scanning: AtomicBool,
    tailscale: Mutex<Option<crate::model::Vpn>>,
    /// JSON of the last snapshot sent, to skip identical ones.
    last_sent: Mutex<String>,
    /// Set by the signal reader whenever NetworkManager reports a change.
    changed: Notify,
}

#[derive(Clone)]
struct Backend {
    conn: Connection,
    events: Tx,
    shared: Arc<Shared>,
}

async fn run(mut cmds: Rx, events: Tx) {
    let conn = match Connection::system().await {
        Ok(conn) => conn,
        Err(e) => return offline(cmds, events, &e).await,
    };
    let backend = Backend {
        conn,
        events,
        shared: Arc::new(Shared {
            scanning: AtomicBool::new(false),
            tailscale: Mutex::new(None),
            last_sent: Mutex::new(String::new()),
            changed: Notify::new(),
        }),
    };
    backend.refresh_tailscale().await;
    backend.publish().await;
    tokio::spawn(backend.clone().watch());
    tokio::spawn({
        let backend = backend.clone();
        async move {
            let _ = backend.rescan().await;
        }
    });
    while let Some(cmd) = cmds.recv().await {
        tokio::spawn(backend.clone().handle(cmd));
    }
}

/// No system bus: show an empty system and fail every action with a reason.
async fn offline(mut cmds: Rx, events: Tx, error: &zbus::Error) {
    let message = format!("Can't reach the system D-Bus ({error}). Is NetworkManager running?");
    let _ = events.send(Event::Snapshot(Snapshot::default()));
    while let Some(cmd) = cmds.recv().await {
        match cmd {
            Cmd::RevealPassword { ssid } => {
                let password = Err(message.clone());
                let _ = events.send(Event::Password { ssid, password });
            }
            Cmd::Details => {
                let _ = events.send(Event::Details(Details::default()));
            }
            cmd => send_done(&events, &target(&cmd), Err(message.clone())),
        }
    }
}

/// The row id the UI marks pending for this command.
fn target(cmd: &Cmd) -> String {
    match cmd {
        Cmd::Join { network, .. } => network.clone(),
        Cmd::Forget { ssid } | Cmd::RevealPassword { ssid } => ssid.clone(),
        Cmd::SetIpv4 { interface, .. } => interface.clone(),
        Cmd::SetVpn { vpn, .. } => vpn.clone(),
        Cmd::SetAutoJoin { ssid, .. } => ssid.clone(),
        Cmd::Rescan | Cmd::SetWifiPower(_) | Cmd::Details | Cmd::RequestLocation => {
            "wifi".to_string()
        }
    }
}

fn send_done(events: &Tx, target: &str, result: Result<String, String>) {
    let _ = events.send(Event::Done {
        target: target.to_string(),
        result,
    });
}

impl Backend {
    async fn handle(self, cmd: Cmd) {
        let target = target(&cmd);
        let result = match cmd {
            Cmd::Rescan => self.rescan().await,
            Cmd::Join { network, password } => {
                wifi::join(&self.conn, &network, password.as_deref()).await
            }
            Cmd::Forget { ssid } => wifi::forget(&self.conn, &ssid).await,
            Cmd::SetWifiPower(on) => wifi::set_power(&self.conn, on).await,
            Cmd::SetAutoJoin { ssid, on } => wifi::set_auto_join(&self.conn, &ssid, on).await,
            Cmd::SetIpv4 { interface, config } => {
                interfaces::set_ipv4(&self.conn, &interface, &config).await
            }
            Cmd::SetVpn { vpn, on } => self.set_vpn(&vpn, on).await,
            Cmd::Details => {
                let details = wifi::details(&self.conn).await;
                let _ = self.events.send(Event::Details(details));
                return;
            }
            Cmd::RevealPassword { ssid } => {
                let password = wifi::reveal_password(&self.conn, &ssid).await;
                let _ = self.events.send(Event::Password { ssid, password });
                return;
            }
            Cmd::RequestLocation => Ok("Network names are always visible on Linux.".to_string()),
        };
        // Send the new state first so the row never shows stale data after "done".
        self.publish().await;
        send_done(&self.events, &target, result);
    }

    async fn set_vpn(&self, vpn: &str, on: bool) -> Result<String, String> {
        if vpn != vpn::TAILSCALE_ID {
            return vpn::set_network_manager(&self.conn, vpn, on).await;
        }
        let result = vpn::set_tailscale(on).await;
        self.refresh_tailscale().await;
        result
    }

    async fn rescan(&self) -> Result<String, String> {
        self.shared.scanning.store(true, Ordering::Relaxed);
        self.publish().await;
        let result = wifi::scan(&self.conn).await;
        self.shared.scanning.store(false, Ordering::Relaxed);
        self.publish().await;
        result.map(|()| "Scan finished.".to_string())
    }

    async fn refresh_tailscale(&self) {
        *self.shared.tailscale.lock().await = vpn::tailscale().await;
    }

    async fn snapshot(&self) -> zbus::Result<Snapshot> {
        let (mut interfaces, primary) = interfaces::list(&self.conn).await?;
        let wifi = wifi::read(&self.conn, self.shared.scanning.load(Ordering::Relaxed)).await?;
        if let Some(wifi) = wifi.as_ref().filter(|w| !w.power)
            && let Some(interface) = interfaces.iter_mut().find(|i| i.id == wifi.interface)
        {
            interface.summary = "off".to_string();
        }
        let mut vpns: Vec<_> = self
            .shared
            .tailscale
            .lock()
            .await
            .clone()
            .into_iter()
            .collect();
        vpns.extend(vpn::network_manager(&self.conn).await.unwrap_or_default());
        Ok(Snapshot {
            interfaces,
            wifi,
            vpns,
            primary,
            caps: Caps {
                edit_ip: true,
                edit_needs_admin: false,
                reveal_touch_id: false,
            },
        })
    }

    /// Send a snapshot unless it is identical to the last one.
    async fn publish(&self) {
        let Ok(snapshot) = self.snapshot().await else {
            return;
        };
        let Ok(json) = serde_json::to_string(&snapshot) else {
            return;
        };
        let mut last = self.shared.last_sent.lock().await;
        if *last != json {
            *last = json;
            let _ = self.events.send(Event::Snapshot(snapshot));
        }
    }

    /// Rebuild when NetworkManager reports a change, and poll Tailscale.
    async fn watch(self) {
        tokio::spawn(self.clone().read_signals());
        let mut poll = interval_at(Instant::now() + TAILSCALE_POLL, TAILSCALE_POLL);
        loop {
            tokio::select! {
                _ = self.shared.changed.notified() => {}
                _ = poll.tick() => self.refresh_tailscale().await,
            }
            sleep(DEBOUNCE).await;
            self.publish().await;
        }
    }

    /// Only drains signals, never awaits anything else: zbus stops
    /// delivering replies on this connection while a stream's queue is full.
    async fn read_signals(self) {
        let Ok(mut signals) = nm_signals(&self.conn).await else {
            return;
        };
        while signals.next().await.is_some() {
            self.shared.changed.notify_one();
        }
    }
}

/// Every signal from any NetworkManager object: property changes, access
/// points coming and going, state changes, saved connections changing.
async fn nm_signals(conn: &Connection) -> zbus::Result<MessageStream> {
    let rule = MatchRule::builder()
        .msg_type(Type::Signal)
        .path_namespace("/org/freedesktop/NetworkManager")?
        .build();
    MessageStream::for_match_rule(rule, conn, None).await
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn next(events: &mut tokio::sync::mpsc::UnboundedReceiver<Event>) -> Option<Event> {
        tokio::time::timeout(Duration::from_secs(20), events.recv())
            .await
            .unwrap()
    }

    /// Needs a real NetworkManager. Read-only except for a Wi-Fi scan:
    /// `cargo test -p telmo-net -- --ignored --nocapture live`
    #[tokio::test(flavor = "current_thread")]
    #[ignore]
    async fn live_snapshot_rescan_details() {
        let (cmd_tx, cmd_rx) = tokio::sync::mpsc::unbounded_channel();
        let (event_tx, mut events) = tokio::sync::mpsc::unbounded_channel();
        spawn(cmd_rx, event_tx);
        cmd_tx.send(Cmd::Rescan).unwrap();
        cmd_tx.send(Cmd::Details).unwrap();
        let mut seen_done = false;
        let mut seen_details = false;
        while !(seen_done && seen_details) {
            let event = tokio::time::timeout(Duration::from_secs(20), events.recv())
                .await
                .unwrap()
                .unwrap();
            match event {
                Event::Snapshot(s) => {
                    println!("snapshot: scanning={:?}", s.wifi.map(|w| w.scanning))
                }
                Event::Done { target, result } => {
                    println!("done {target}: {result:?}");
                    seen_done = true;
                }
                Event::Details(d) => {
                    println!("details: {d:?}");
                    seen_details = true;
                }
                _ => {}
            }
        }
    }

    /// Turns the Wi-Fi radio off and back on, checking snapshots follow.
    #[tokio::test(flavor = "current_thread")]
    #[ignore]
    async fn live_wifi_power_toggle() {
        let (cmd_tx, cmd_rx) = tokio::sync::mpsc::unbounded_channel();
        let (event_tx, mut events) = tokio::sync::mpsc::unbounded_channel();
        spawn(cmd_rx, event_tx);
        let mut power_seen = Vec::new();
        let start = loop {
            if let Some(Event::Snapshot(s)) = next(&mut events).await {
                break s.wifi.map(|w| w.power).unwrap_or(false);
            }
        };
        for on in [!start, start] {
            cmd_tx.send(Cmd::SetWifiPower(on)).unwrap();
            loop {
                match next(&mut events).await.unwrap() {
                    Event::Snapshot(s) => {
                        println!("snap power {:?}", s.wifi.as_ref().map(|w| w.power));
                        power_seen.push(s.wifi.map(|w| w.power))
                    }
                    Event::Done { result, .. } => {
                        println!("power {on}: {result:?}");
                        break;
                    }
                    _ => {}
                }
            }
        }
        println!("snapshots saw power: {power_seen:?}");
        assert!(power_seen.contains(&Some(!start)));
    }
}
