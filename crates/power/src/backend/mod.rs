//! The UI talks to a backend only through `Cmd` and `Event`.

use crate::model::{AwakeChoice, EnergyUser, PowerMode, Snapshot};
use tokio::{
    sync::mpsc::{UnboundedReceiver, UnboundedSender},
    task::JoinHandle,
};

#[cfg(any(target_os = "linux", target_os = "macos"))]
mod awake;
#[cfg(target_os = "linux")]
pub mod linux;
#[cfg(target_os = "macos")]
pub mod macos;
pub mod mock;

/// `Done::target` for the two settings rows.
pub const MODE: &str = "mode";
pub const AWAKE: &str = "awake";

#[derive(Debug, Clone)]
pub enum Cmd {
    SetMode(PowerMode),
    SetKeepAwake(AwakeChoice),
}

#[derive(Debug, Clone)]
pub enum Event {
    Snapshot(Snapshot),
    /// Sent once, shortly after start, on systems that can list them.
    EnergyUsers(Vec<EnergyUser>),
    /// An action finished; `target` is `MODE` or `AWAKE`.
    Done {
        target: &'static str,
        result: Result<String, String>,
    },
}

pub type Tx = UnboundedSender<Event>;
pub type Rx = UnboundedReceiver<Cmd>;

/// The returned handle finishes once the command channel has closed.
pub fn spawn(mock: bool, cmds: Rx, events: Tx) -> Option<JoinHandle<()>> {
    if mock {
        return Some(mock::spawn(cmds, events));
    }
    #[cfg(target_os = "linux")]
    return Some(linux::spawn(cmds, events));
    #[cfg(target_os = "macos")]
    return Some(macos::spawn(cmds, events));
}
