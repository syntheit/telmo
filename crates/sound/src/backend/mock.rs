//! Fake system for `--mock`, UI work and tests.

use super::{Event, Rx, Tx};

pub fn spawn(_cmds: Rx, events: Tx) {
    let _ = events.send(Event::Snapshot(Default::default()));
}
