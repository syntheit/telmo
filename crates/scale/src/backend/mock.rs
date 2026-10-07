//! Fake trackpad for `--mock`, UI work and tests: a value that settles to 124.5 g.

use super::{Event, Tx};
use crate::model::Snapshot;
use std::time::Duration;
use tokio::{task::JoinHandle, time::sleep};

const STEP: Duration = Duration::from_millis(250);
const SETTLE: [f32; 8] = [0.0, 31.0, 88.0, 117.0, 126.8, 123.9, 124.7, 124.5];

pub fn snapshot(grams: f32, stable: bool) -> Snapshot {
    Snapshot {
        unavailable: None,
        touching: true,
        grams,
        stable,
    }
}

pub fn idle() -> Snapshot {
    Snapshot::default()
}

pub fn spawn(events: Tx) -> JoinHandle<()> {
    tokio::spawn(async move {
        if events.send(Event::Snapshot(idle())).is_err() {
            return;
        }
        sleep(STEP * 2).await;
        for grams in SETTLE {
            if events
                .send(Event::Snapshot(snapshot(grams, false)))
                .is_err()
            {
                return;
            }
            sleep(STEP).await;
        }
        sleep(STEP * 4).await;
        let _ = events.send(Event::Snapshot(snapshot(124.5, true)));
        events.closed().await;
    })
}
