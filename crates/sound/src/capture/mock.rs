//! A moving fake spectrum for `--mock`, and a fixed one for snapshot tests.

use super::{FRAME, stopped};
use crate::backend::{Event, Tx};
use crate::spectrum::BARS;
use std::{sync::atomic::AtomicBool, thread};

pub fn run(stop: &AtomicBool, events: &Tx) {
    let mut t = 0.0;
    while !stopped(stop) {
        if events.send(Event::Spectrum(canned_at(t))).is_err() {
            return;
        }
        t += FRAME.as_secs_f32();
        thread::sleep(FRAME);
    }
}

/// Bass-heavy hills that drift with `t`, so the picture is the same for the
/// same `t`.
pub fn canned_at(t: f32) -> Vec<f32> {
    (0..BARS)
        .map(|i| {
            let x = i as f32 / BARS as f32;
            let body = 0.85 - 0.55 * x;
            let wave = 0.5 + 0.5 * (x * 23.0 - t * 5.0).sin() * (x * 7.0 + t * 2.0).cos();
            (body * wave + 0.08).clamp(0.0, 1.0)
        })
        .collect()
}
