//! The UI talks to a backend only through `Cmd` and `Event`.

use crate::model::Snapshot;
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender};

#[cfg(target_os = "linux")]
pub mod linux;
#[cfg(target_os = "macos")]
pub mod macos;
pub mod mock;

#[derive(Debug, Clone)]
pub enum Cmd {
    Brightness { display: String, value: f32 },
    Night { on: bool, warmth: f32 },
    Size { display: String, mode: String },
}

#[derive(Debug, Clone)]
pub enum Event {
    Snapshot(Snapshot),
    /// Only failures are reported; successes show up in the next snapshot.
    Failed(String),
}

/// Said under the scale choices: `hyprctl keyword` does not outlive a reload.
pub const SCALE_NOTE: &str = "Resets on Hyprland reload. Save it in hyprland.conf to keep it.";

pub type Tx = UnboundedSender<Event>;
pub type Rx = UnboundedReceiver<Cmd>;

pub fn spawn(mock: bool, cmds: Rx, events: Tx) {
    if mock {
        return mock::spawn(cmds, events);
    }
    #[cfg(target_os = "linux")]
    linux::spawn(cmds, events);
    #[cfg(target_os = "macos")]
    macos::spawn(cmds, events);
}
