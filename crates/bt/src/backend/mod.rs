//! The UI talks to a backend only through `Cmd` and `Event`.

use crate::model::{PairPrompt, Snapshot};
use tokio::{
    sync::mpsc::{UnboundedReceiver, UnboundedSender},
    task::JoinHandle,
};

#[cfg(target_os = "linux")]
pub mod linux;
#[cfg(target_os = "macos")]
pub mod macos;
pub mod mock;

#[derive(Debug, Clone)]
pub enum Cmd {
    SetPower(bool),
    Connect(String),
    Disconnect(String),
    StartScan,
    StopScan,
    Pair(String),
    /// Answer to a `PairPrompt`: Some(pin) for EnterPin, None to reject,
    /// Some("") to accept a confirmation or passkey display.
    PairReply(Option<String>),
    SetAutoConnect(String, bool),
    Forget(String),
    Rename(String, String),
}

#[derive(Debug, Clone)]
pub enum Event {
    Snapshot(Snapshot),
    /// An action finished; `target` is the device id (or "adapter").
    Done {
        target: String,
        result: Result<String, String>,
    },
    Pairing {
        device: String,
        prompt: PairPrompt,
    },
}

pub type Tx = UnboundedSender<Event>;
pub type Rx = UnboundedReceiver<Cmd>;

/// The returned handle, if any, finishes once the backend has cleaned up after
/// the command channel closed.
pub fn spawn(mock: bool, cmds: Rx, events: Tx) -> Option<JoinHandle<()>> {
    if mock {
        return Some(mock::spawn(cmds, events));
    }
    #[cfg(target_os = "linux")]
    return Some(linux::spawn(cmds, events));
    #[cfg(target_os = "macos")]
    {
        macos::spawn(cmds, events);
        None
    }
}
