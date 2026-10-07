//! Turns raw samples into bar heights: mono mixdown, Hann window, FFT, log
//! spaced bins, dB, then per-bar whitening and an auto-sensitivity.
//! Pure, so it is unit tested.
//!
//! The adaptive normalization follows cava (https://github.com/karlstav/cava,
//! MIT, see cavacore.c): bars show energy relative to what each bar has been
//! doing lately, and a global sensitivity keeps the tallest bars near the top.

use rustfft::{Fft, FftPlanner, num_complex::Complex};
use std::{f32::consts::PI, sync::Arc};

pub const BARS: usize = 64;
pub const FFT_SIZE: usize = 2048;
const LOW_HZ: f32 = 40.0;
const HIGH_HZ: f32 = 16_000.0;
/// What maps to 0.0 and 1.0 after the tilt below. Wide, so quiet passages
/// sit visibly lower and loud ones reach the top.
const FLOOR_DB: f32 = -80.0;
const CEIL_DB: f32 = -10.0;
/// Music is loudest in the bass; lift the highs so the right side moves too.
const TILT_DB_PER_OCTAVE: f32 = 2.5;
/// Per frame: how fast each bar's running average follows its energy, and
/// how fast its running peak sinks toward that average.
const AVERAGE_RATE: f32 = 0.03;
const PEAK_SINK: f32 = 0.01;
/// Smallest peak-to-average distance, so a flat tone doesn't blow up noise.
const MIN_RANGE: f32 = 0.1;
/// Where a bar sits while it matches its own recent average.
const SETTLE: f32 = 0.3;
/// Share of the whitened value in the output; the rest is plain energy, so
/// loud bars stay taller than quiet ones.
const WHITE_SHARE: f32 = 0.7;
/// Energy below this fades the whitened part out, so silence stays at zero.
const GATE: f32 = 0.15;
/// Auto-sensitivity: the tallest bar should land just under the top.
const TARGET: f32 = 0.9;
const SENS_RISE: f32 = 1.003;
const SENS_FALL: f32 = 0.95;
const SENS_MIN: f32 = 0.3;
const SENS_MAX: f32 = 4.0;
/// Frames quieter than this don't raise the sensitivity.
const QUIET: f32 = 0.05;
/// Mild curve on the final height: quiet is lower, peaks keep their reach.
const GAMMA: f32 = 1.25;

pub struct Analyzer {
    bin_hz: f32,
    fft: Arc<dyn Fft<f32>>,
    window: Vec<f32>,
    /// The latest `FFT_SIZE` mono samples, oldest first.
    mono: Vec<f32>,
    buffer: Vec<Complex<f32>>,
    /// Global auto-sensitivity.
    sensitivity: f32,
    /// Each bar's slow average and slowly sinking peak of energy.
    average: Vec<f32>,
    peak: Vec<f32>,
}

impl Analyzer {
    pub fn new(sample_rate: f32) -> Self {
        let window = (0..FFT_SIZE)
            .map(|n| 0.5 - 0.5 * (2.0 * PI * n as f32 / FFT_SIZE as f32).cos())
            .collect();
        Self {
            bin_hz: sample_rate / FFT_SIZE as f32,
            fft: FftPlanner::new().plan_fft_forward(FFT_SIZE),
            window,
            mono: vec![0.0; FFT_SIZE],
            buffer: vec![Complex::default(); FFT_SIZE],
            sensitivity: 1.0,
            average: vec![0.0; BARS],
            peak: vec![0.0; BARS],
        }
    }

    /// Adds interleaved samples, mixed down to mono.
    pub fn push(&mut self, samples: &[f32], channels: usize) {
        let channels = channels.max(1);
        let mono: Vec<f32> = samples
            .chunks_exact(channels)
            .map(|frame| frame.iter().sum::<f32>() / channels as f32)
            .collect();
        let keep = mono.len().min(FFT_SIZE);
        self.mono.copy_within(keep.., 0);
        self.mono[FFT_SIZE - keep..].copy_from_slice(&mono[mono.len() - keep..]);
    }

    /// Bar heights from 0.0 to 1.0 for the samples pushed so far.
    pub fn frame(&mut self) -> Vec<f32> {
        for (slot, (sample, w)) in self
            .buffer
            .iter_mut()
            .zip(self.mono.iter().zip(&self.window))
        {
            *slot = Complex::new(sample * w, 0.0);
        }
        self.fft.process(&mut self.buffer);
        // A full-scale sine under a Hann window peaks at FFT_SIZE / 4.
        let scale = 4.0 / FFT_SIZE as f32;
        let magnitudes: Vec<f32> = self.buffer[..FFT_SIZE / 2]
            .iter()
            .map(|c| c.norm() * scale)
            .collect();
        let energy: Vec<f32> = (0..BARS).map(|i| self.bar(&magnitudes, i)).collect();
        let bars = self.whiten(&energy);
        self.apply_sensitivity(bars)
    }

    fn bar(&self, magnitudes: &[f32], i: usize) -> f32 {
        let lo = edge(i) / self.bin_hz;
        let hi = edge(i + 1) / self.bin_hz;
        let amplitude = if hi - lo < 1.0 {
            interpolate(magnitudes, (lo + hi) / 2.0)
        } else {
            let first = lo.ceil() as usize;
            let last = (hi.floor() as usize).min(magnitudes.len() - 1);
            magnitudes[first..=last].iter().copied().fold(0.0, f32::max)
        };
        let center = (edge(i) * edge(i + 1)).sqrt();
        let db = 20.0 * (amplitude + 1e-9).log10() + TILT_DB_PER_OCTAVE * (center / 1000.0).log2();
        ((db - FLOOR_DB) / (CEIL_DB - FLOOR_DB)).clamp(0.0, 1.0)
    }

    /// Shows each bar's energy relative to its own recent average and peak:
    /// a steady tone settles low, a hit or a change jumps.
    fn whiten(&mut self, energy: &[f32]) -> Vec<f32> {
        let mut bars = Vec::with_capacity(BARS);
        for (i, e) in energy.iter().copied().enumerate() {
            if self.average[i] == 0.0 {
                self.average[i] = e;
            }
            let (average, peak) = (self.average[i], self.peak[i].max(e));
            bars.push(whitened(e, average, peak));
            self.average[i] += (e - average) * AVERAGE_RATE;
            self.peak[i] = peak - (peak - self.average[i]) * PEAK_SINK;
        }
        bars
    }

    /// Overshoot cuts the sensitivity fast, a quiet stretch raises it slowly.
    fn apply_sensitivity(&mut self, bars: Vec<f32>) -> Vec<f32> {
        let tallest = bars.iter().copied().fold(0.0, f32::max) * self.sensitivity;
        self.sensitivity = next_sensitivity(self.sensitivity, tallest);
        bars.iter()
            .map(|b| (b * self.sensitivity).clamp(0.0, 1.0).powf(GAMMA))
            .collect()
    }
}

/// One bar's height before sensitivity, from its energy `e` and its running
/// `average` and `peak`. Zero for silence.
pub fn whitened(e: f32, average: f32, peak: f32) -> f32 {
    let range = (peak - average).max(MIN_RANGE);
    let relative = (SETTLE + (e - average) / range).max(0.0);
    let mixed = WHITE_SHARE * relative + (1.0 - WHITE_SHARE) * e;
    mixed * (e / GATE).min(1.0)
}

/// The sensitivity after a frame whose tallest bar came out at `tallest`.
pub fn next_sensitivity(sensitivity: f32, tallest: f32) -> f32 {
    let next = if tallest > 1.0 {
        sensitivity * SENS_FALL
    } else if tallest > QUIET && tallest < TARGET {
        sensitivity * SENS_RISE
    } else {
        sensitivity
    };
    next.clamp(SENS_MIN, SENS_MAX)
}

/// Frequency where bar `i` starts. Bars are evenly spaced on a log scale.
fn edge(i: usize) -> f32 {
    LOW_HZ * (HIGH_HZ / LOW_HZ).powf(i as f32 / BARS as f32)
}

fn interpolate(values: &[f32], position: f32) -> f32 {
    let i = (position.floor() as usize).min(values.len() - 2);
    let t = (position - i as f32).clamp(0.0, 1.0);
    values[i] * (1.0 - t) + values[i + 1] * t
}

#[cfg(test)]
mod tests {
    use super::*;

    const RATE: f32 = 48_000.0;

    fn sine(hz: f32, amplitude: f32, frames: usize) -> Vec<f32> {
        (0..frames * 2)
            .map(|n| amplitude * (2.0 * PI * hz * (n / 2) as f32 / RATE).sin())
            .collect()
    }

    /// The bars after the running average has caught up with a steady sound.
    fn settled(analyzer: &mut Analyzer) -> Vec<f32> {
        (0..20).map(|_| analyzer.frame()).last().unwrap_or_default()
    }

    fn loudest(bars: &[f32]) -> usize {
        let mut best = 0;
        for (i, bar) in bars.iter().enumerate() {
            if *bar > bars[best] {
                best = i;
            }
        }
        best
    }

    fn bar_for(hz: f32) -> usize {
        (0..BARS).find(|i| edge(i + 1) > hz).unwrap_or(BARS - 1)
    }

    #[test]
    fn silence_is_flat() {
        let mut analyzer = Analyzer::new(RATE);
        analyzer.push(&vec![0.0; FFT_SIZE * 2], 2);
        assert!(analyzer.frame().iter().all(|b| *b == 0.0));
    }

    #[test]
    fn a_tone_lights_its_own_bar() {
        for hz in [100.0, 440.0, 1000.0, 5000.0, 12_000.0] {
            let mut analyzer = Analyzer::new(RATE);
            analyzer.push(&sine(hz, 0.5, FFT_SIZE), 2);
            let bars = settled(&mut analyzer);
            let at = loudest(&bars);
            assert!(at.abs_diff(bar_for(hz)) <= 1, "{hz} Hz lit bar {at}");
            assert!(bars[at] > 0.25, "{hz} Hz only reached {}", bars[at]);
            let far = bars[(at + BARS / 2) % BARS];
            assert!(far < bars[at] / 2.0, "{hz} Hz leaked to {far}");
        }
    }

    #[test]
    fn bars_are_log_spaced_and_cover_the_range() {
        assert!((edge(0) - LOW_HZ).abs() < 0.01);
        assert!((edge(BARS) - HIGH_HZ).abs() < 1.0);
        let ratio = edge(1) / edge(0);
        assert!((edge(BARS / 2 + 1) / edge(BARS / 2) - ratio).abs() < 1e-4);
    }

    #[test]
    fn auto_gain_lifts_quiet_music_but_never_clips() {
        let mut analyzer = Analyzer::new(RATE);
        let quiet = sine(1000.0, 0.01, FFT_SIZE);
        analyzer.push(&quiet, 2);
        let first = analyzer.frame()[bar_for(1000.0)];
        let mut last = first;
        for _ in 0..100 {
            last = analyzer.frame()[bar_for(1000.0)];
        }
        assert!(last > first);
        analyzer.push(&sine(1000.0, 1.0, FFT_SIZE), 2);
        assert!(
            settled(&mut analyzer)
                .iter()
                .all(|b| (0.0..=1.0).contains(b))
        );
    }

    /// Pushes one 30 fps frame of a 1 kHz tone and returns the bars.
    fn step(analyzer: &mut Analyzer, amplitude: f32, n: usize) -> Vec<f32> {
        let frame = 1600;
        let start = n * frame;
        let samples: Vec<f32> = (0..frame * 2)
            .map(|k| {
                let t = (start + k / 2) as f32 / RATE;
                amplitude * (2.0 * PI * 1000.0 * t).sin()
            })
            .collect();
        analyzer.push(&samples, 2);
        analyzer.frame()
    }

    #[test]
    fn bursts_jump_and_the_tone_settles_back_down() {
        let mut analyzer = Analyzer::new(RATE);
        let at = bar_for(1000.0);
        let mut n = 0;
        for cycle in 0..5 {
            let mut calm = 0.0;
            for _ in 0..45 {
                calm = step(&mut analyzer, 0.1, n)[at];
                n += 1;
            }
            let mut burst: f32 = 0.0;
            for _ in 0..6 {
                burst = burst.max(step(&mut analyzer, 0.7, n)[at]);
                n += 1;
            }
            if cycle >= 1 {
                assert!(burst - calm > 0.35, "burst {burst} vs calm {calm}");
                assert!(calm < 0.6, "steady tone sits at {calm}");
            }
        }
    }

    #[test]
    fn a_steady_tone_settles_low() {
        let mut analyzer = Analyzer::new(RATE);
        let at = bar_for(1000.0);
        let steady = (0..90)
            .map(|n| step(&mut analyzer, 0.3, n)[at])
            .last()
            .unwrap_or_default();
        assert!(steady < 0.7, "{steady}");
    }

    #[test]
    fn whitening_is_zero_for_silence_and_grows_with_surprise() {
        assert_eq!(whitened(0.0, 0.0, 0.0), 0.0);
        let steady = whitened(0.5, 0.5, 0.5);
        assert!(whitened(0.8, 0.5, 0.8) > steady + 0.3);
        assert!(whitened(0.2, 0.5, 0.8) < steady);
    }

    #[test]
    fn sensitivity_drops_fast_and_climbs_slowly() {
        let down = 1.0 - next_sensitivity(1.0, 1.2);
        let up = next_sensitivity(1.0, 0.5) - 1.0;
        assert!(down > 10.0 * up && up > 0.0);
        assert_eq!(next_sensitivity(1.0, 0.0), 1.0);
        assert_eq!(next_sensitivity(SENS_MAX, 0.5), SENS_MAX);
    }

    #[test]
    fn mono_mixdown_averages_channels() {
        let mut analyzer = Analyzer::new(RATE);
        // Opposite channels cancel out.
        let left = sine(1000.0, 0.5, FFT_SIZE);
        let opposite: Vec<f32> = left.chunks(2).flat_map(|f| [f[0], -f[0]]).collect();
        analyzer.push(&opposite, 2);
        assert!(analyzer.frame().iter().all(|b| *b == 0.0));
    }
}
