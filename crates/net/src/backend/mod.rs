//! The UI talks to a backend only through `Cmd` and `Event`. Each platform
//! backend runs on its own task or thread and owns all system access.

use crate::model::{Details, Ipv4Config, Snapshot};
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender};

#[cfg(target_os = "linux")]
pub mod linux;
#[cfg(target_os = "macos")]
pub mod macos;
pub mod mock;

#[derive(Debug, Clone)]
pub enum Cmd {
    Rescan,
    Join {
        network: String,
        password: Option<String>,
    },
    Forget {
        ssid: String,
    },
    SetWifiPower(bool),
    SetAutoJoin {
        ssid: String,
        on: bool,
    },
    SetIpv4 {
        interface: String,
        config: Ipv4Config,
    },
    SetVpn {
        vpn: String,
        on: bool,
    },
    /// Fetch `Details` for the connected Wi-Fi network.
    Details,
    RevealPassword {
        ssid: String,
    },
    /// macOS only: ask for Location so network names are visible.
    RequestLocation,
}

#[derive(Debug, Clone)]
pub enum Event {
    Snapshot(Snapshot),
    /// An action finished. `target` is the id it acted on (network, vpn,
    /// interface) so the UI can clear that row's pending state.
    Done {
        target: String,
        result: Result<String, String>,
    },
    Details(Details),
    Password {
        ssid: String,
        password: Result<String, String>,
    },
    /// Speedtest progress. The app starts the test itself (see telmo-speed).
    Speed(telmo_speed::Update),
}

pub type Tx = UnboundedSender<Event>;
pub type Rx = UnboundedReceiver<Cmd>;

/// Start the real backend for this OS, or the mock.
pub fn spawn(mock: bool, cmds: Rx, events: Tx) {
    if mock {
        return mock::spawn(cmds, events);
    }
    #[cfg(target_os = "linux")]
    linux::spawn(cmds, events);
    #[cfg(target_os = "macos")]
    macos::spawn(cmds, events);
}
