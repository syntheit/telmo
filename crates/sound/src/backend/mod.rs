//! The UI talks to a backend only through `Cmd` and `Event`.

use crate::model::{Direction, Snapshot, Target};
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender};

#[cfg(target_os = "linux")]
pub mod linux;
#[cfg(target_os = "macos")]
pub mod macos;
pub mod mock;

#[derive(Debug, Clone)]
pub enum Cmd {
    SetDefault(Direction, String),
    SetVolume(Target, f32),
    SetMute(Target, bool),
    MoveStream { stream: String, device: String },
    SetProfile { device: String, profile: String },
}

#[derive(Debug, Clone)]
pub enum Event {
    Snapshot(Snapshot),
    /// Only failures are reported; successes show up in the next snapshot.
    Failed(String),
}

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
