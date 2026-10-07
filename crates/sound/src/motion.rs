//! What the visualizer shows, chasing the latest spectrum every frame.
//! Spectrum frames only set targets. Bars snap up and fall with gravity
//! (accelerating), the way cava does it (https://github.com/karlstav/cava).

use crate::spectrum::BARS;

/// Frames a second the constants below are tuned for.
pub const FPS: f32 = 30.0;
/// Share of the gap a bar closes per frame when rising.
const ATTACK: f32 = 0.8;
/// Falling starts at this speed (bar heights per frame) and speeds up by
/// `GRAVITY` every frame, so a bar drops slowly at first, then quickly.
const FALL_START: f32 = 0.01;
const GRAVITY: f32 = 0.006;
/// Share of a bar's height its neighbours are lifted to, per step away.
const MONSTERCAT: f32 = 0.5;
/// Frames a peak cap stays put, then how fast it drops per frame.
const PEAK_HOLD: f32 = 12.0;
const PEAK_FALL: f32 = 0.012;
/// Hue degrees the rainbow drifts each second.
const DRIFT_PER_SECOND: f32 = 18.0;
/// Below this a bar counts as zero.
const EPSILON: f32 = 0.004;

pub struct Motion {
    pub bars: Vec<f32>,
    pub peaks: Vec<f32>,
    hold: Vec<f32>,
    /// Current fall speed of each bar.
    speed: Vec<f32>,
    target: Vec<f32>,
    /// Hue offset in degrees; the whole gradient slides by this much.
    pub phase: f32,
}

impl Motion {
    pub fn new() -> Self {
        Self {
            bars: vec![0.0; BARS],
            peaks: vec![0.0; BARS],
            hold: vec![0.0; BARS],
            speed: vec![FALL_START; BARS],
            target: vec![0.0; BARS],
            phase: 0.0,
        }
    }

    pub fn set_target(&mut self, frame: &[f32]) {
        self.target = monstercat(frame);
    }

    pub fn silence(&mut self) {
        self.target.fill(0.0);
    }

    /// Advances by `frames` (1.0 is one frame at `FPS`).
    pub fn step(&mut self, frames: f32) {
        for i in 0..self.bars.len() {
            (self.bars[i], self.speed[i]) =
                ease(self.bars[i], self.speed[i], self.target[i], frames);
            (self.peaks[i], self.hold[i]) =
                step_peak(self.peaks[i], self.hold[i], self.bars[i], frames);
        }
        self.phase = (self.phase + DRIFT_PER_SECOND * frames / FPS) % 360.0;
    }

    /// Nothing is lit and nothing will move, so no more frames are needed.
    pub fn settled(&self) -> bool {
        self.bars.iter().chain(&self.peaks).all(|v| *v == 0.0)
            && self.target.iter().all(|v| *v == 0.0)
    }

    pub fn lit(&self) -> bool {
        self.bars.iter().any(|b| *b > 0.0)
    }

    /// Jumps straight to `levels`, for tests of drawing.
    #[cfg(test)]
    pub fn show(&mut self, levels: &[f32]) {
        self.bars = levels.to_vec();
        self.peaks = levels.to_vec();
        self.target = levels.to_vec();
    }
}

/// Moves `current` toward `target`: snaps up, falls with gravity. Returns the
/// new height and fall speed.
pub fn ease(current: f32, speed: f32, target: f32, frames: f32) -> (f32, f32) {
    if target > current {
        let next = current + (target - current) * (1.0 - (1.0 - ATTACK).powf(frames));
        return (next, FALL_START);
    }
    let fall = speed * frames + 0.5 * GRAVITY * frames * frames;
    let next = (current - fall).max(target);
    let next = if next < EPSILON && target == 0.0 {
        0.0
    } else {
        next
    };
    (next, speed + GRAVITY * frames)
}

/// A peak cap: follows its bar up, waits, then sinks, never below the bar.
/// Returns the new cap height and the frames of waiting left.
pub fn step_peak(peak: f32, hold: f32, bar: f32, frames: f32) -> (f32, f32) {
    if bar >= peak {
        (bar, PEAK_HOLD)
    } else if hold > 0.0 {
        (peak, (hold - frames).max(0.0))
    } else {
        let next = (peak - PEAK_FALL * frames).max(bar);
        (if next < EPSILON { 0.0 } else { next }, 0.0)
    }
}

/// Subtle monstercat filter: each bar lifts its neighbours to a fraction of
/// its own height, halving per step, so single spikes grow skirts.
pub fn monstercat(values: &[f32]) -> Vec<f32> {
    let mut out = values.to_vec();
    for (i, value) in values.iter().enumerate() {
        for (j, slot) in out.iter_mut().enumerate().skip(i.saturating_sub(3)).take(7) {
            *slot = slot.max(value * MONSTERCAT.powi(j.abs_diff(i) as i32));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snaps_up_and_falls_with_gravity() {
        let (up, _) = ease(0.0, FALL_START, 1.0, 1.0);
        assert!(up > 0.7);
        let (mut bar, mut speed) = (1.0, FALL_START);
        let mut drops = Vec::new();
        for _ in 0..4 {
            let before = bar;
            (bar, speed) = ease(bar, speed, 0.0, 1.0);
            drops.push(before - bar);
        }
        assert!(drops[0] < 0.05 && drops.windows(2).all(|d| d[1] > d[0]));
    }

    #[test]
    fn easing_never_overshoots_and_reaches_zero() {
        assert_eq!(ease(0.5, FALL_START, 0.5, 1.0).0, 0.5);
        assert!(ease(0.0, FALL_START, 1.0, 100.0).0 <= 1.0);
        let (mut v, mut speed) = (1.0, FALL_START);
        for _ in 0..200 {
            (v, speed) = ease(v, speed, 0.0, 1.0);
        }
        assert_eq!(v, 0.0);
    }

    #[test]
    fn two_half_steps_match_one_whole_when_rising() {
        let whole = ease(0.0, FALL_START, 1.0, 1.0).0;
        let half = ease(0.0, FALL_START, 1.0, 0.5);
        let halves = ease(half.0, half.1, 1.0, 0.5).0;
        assert!((whole - halves).abs() < 1e-5);
    }

    #[test]
    fn peak_holds_then_falls_but_not_below_the_bar() {
        let (mut peak, mut hold) = step_peak(0.0, 0.0, 0.8, 1.0);
        assert_eq!((peak, hold), (0.8, PEAK_HOLD));
        for _ in 0..PEAK_HOLD as usize {
            (peak, hold) = step_peak(peak, hold, 0.2, 1.0);
            assert_eq!(peak, 0.8);
        }
        (peak, hold) = step_peak(peak, hold, 0.2, 1.0);
        assert!(peak < 0.8 && peak > 0.7);
        for _ in 0..100 {
            (peak, hold) = step_peak(peak, hold, 0.2, 1.0);
        }
        assert_eq!(peak, 0.2);
    }

    #[test]
    fn monstercat_skirts_a_spike_and_keeps_flat_runs() {
        let out = monstercat(&[0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0]);
        assert_eq!(&out[2..7], &[0.25, 0.5, 1.0, 0.5, 0.25]);
        assert_eq!(monstercat(&[0.6, 0.6, 0.6]), vec![0.6, 0.6, 0.6]);
    }

    #[test]
    fn settles_after_the_sound_stops() {
        let mut motion = Motion::new();
        assert!(motion.settled());
        motion.set_target(&vec![1.0; BARS]);
        motion.step(1.0);
        assert!(!motion.settled() && motion.lit());
        motion.silence();
        for _ in 0..400 {
            motion.step(1.0);
        }
        assert!(motion.settled() && !motion.lit());
    }

    #[test]
    fn the_rainbow_drifts_and_wraps() {
        let mut motion = Motion::new();
        motion.step(FPS);
        assert!((motion.phase - DRIFT_PER_SECOND).abs() < 1e-3);
        motion.step(FPS * 20.0);
        assert!(motion.phase < 360.0);
    }
}
