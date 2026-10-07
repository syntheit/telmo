//! A moving fake spectrum and a quiet pulsing tone for `--mock`, and a fixed
//! spectrum for snapshot tests.

use super::{FRAME, stopped};
use crate::backend::{Event, Tx};
use crate::model::Source;
use crate::spectrum::BARS;
use std::{sync::atomic::AtomicBool, thread};

const RATE: u32 = 16_000;

pub fn run(stop: &AtomicBool, events: &Tx, source: Source) {
    let mut t = 0.0;
    while !stopped(stop) {
        let spectrum = Event::Spectrum(canned_at(t));
        if source == Source::Desktop && events.send(spectrum).is_err() {
            return;
        }
        let samples = Event::Samples {
            source,
            rate: RATE,
            mono: tone_at(t),
        };
        if events.send(samples).is_err() {
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

/// One frame of a 220 Hz tone whose loudness pulses, so the meter moves.
pub fn tone_at(t: f32) -> Vec<f32> {
    let count = (RATE as f32 * FRAME.as_secs_f32()) as usize;
    let loudness = 0.05 + 0.1 * (t * 3.0).sin().abs();
    (0..count)
        .map(|i| {
            let at = t + i as f32 / RATE as f32;
            loudness * (at * 220.0 * std::f32::consts::TAU).sin()
        })
        .collect()
}
