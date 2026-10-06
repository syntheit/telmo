//! Real backend. Not implemented yet: reports an empty system.

use super::{Event, Rx, Tx};

pub fn spawn(_cmds: Rx, events: Tx) {
    let _ = events.send(Event::Snapshot(Default::default()));
}
