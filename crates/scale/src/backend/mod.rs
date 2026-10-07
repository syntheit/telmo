//! The UI talks to a backend only through `Event`.

use crate::model::Snapshot;
use tokio::{sync::mpsc::UnboundedSender, task::JoinHandle};

#[cfg(not(target_os = "macos"))]
pub mod linux;
#[cfg(target_os = "macos")]
pub mod macos;
pub mod mock;

#[derive(Debug, Clone)]
pub enum Event {
    Snapshot(Snapshot),
}

pub type Tx = UnboundedSender<Event>;

/// The returned handle finishes once the backend has stopped the trackpad,
/// which it does as soon as the event receiver is gone.
pub fn spawn(mock: bool, events: Tx) -> JoinHandle<()> {
    if mock {
        return mock::spawn(events);
    }
    #[cfg(target_os = "macos")]
    return macos::spawn(events);
    #[cfg(not(target_os = "macos"))]
    linux::spawn(events)
}
