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
    /// Bar heights (0.0-1.0, low to high pitch) of what is playing, ~30 a second.
    Spectrum(Vec<f32>),
    /// The OS won't let us listen to what is playing (macOS permission).
    #[cfg_attr(not(target_os = "macos"), allow(dead_code))]
    VisualizerBlocked,
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
