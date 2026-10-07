//! There is no Force Touch trackpad to read here.

use super::{Event, Tx};
use crate::model::Snapshot;
use tokio::task::JoinHandle;

pub fn spawn(events: Tx) -> JoinHandle<()> {
    tokio::spawn(async move {
        let snapshot = Snapshot {
            unavailable: Some("Needs a Mac with a Force Touch trackpad.".into()),
            ..Snapshot::default()
        };
        let _ = events.send(Event::Snapshot(snapshot));
        events.closed().await;
    })
}
