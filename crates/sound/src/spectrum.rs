//! Turns raw samples into bar heights: mono mixdown, Hann window, FFT, log
//! spaced bins, dB, then a gentle auto-gain. Pure, so it is unit tested.

use rustfft::{Fft, FftPlanner, num_complex::Complex};
use std::{f32::consts::PI, sync::Arc};

pub const BARS: usize = 64;
pub const FFT_SIZE: usize = 2048;
const LOW_HZ: f32 = 40.0;
const HIGH_HZ: f32 = 16_000.0;
/// What maps to 0.0 and 1.0 after the tilt below.
const FLOOR_DB: f32 = -72.0;
const CEIL_DB: f32 = -8.0;
/// Music is loudest in the bass; lift the highs so the right side moves too.
const TILT_DB_PER_OCTAVE: f32 = 2.5;
const MAX_GAIN: f32 = 4.0;
/// Frames quieter than this don't raise the gain, so silence stays flat.
const QUIET: f32 = 0.08;

pub struct Analyzer {
    bin_hz: f32,
    fft: Arc<dyn Fft<f32>>,
    window: Vec<f32>,
    /// The latest `FFT_SIZE` mono samples, oldest first.
    mono: Vec<f32>,
    buffer: Vec<Complex<f32>>,
    gain: f32,
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
            gain: 1.0,
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
        let bars: Vec<f32> = (0..BARS).map(|i| self.bar(&magnitudes, i)).collect();
        self.apply_gain(bars)
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

    /// Quiet music is lifted, loud music is pulled back. Falls fast, rises slowly.
    fn apply_gain(&mut self, mut bars: Vec<f32>) -> Vec<f32> {
        let peak = bars.iter().copied().fold(0.0, f32::max);
        if peak * self.gain > 0.95 {
            self.gain = (0.95 / peak).max(1.0);
        } else if peak > QUIET {
            self.gain = (self.gain * 1.01).min(MAX_GAIN);
        }
        for bar in &mut bars {
            *bar = (*bar * self.gain).clamp(0.0, 1.0);
        }
        bars
    }
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
            let bars = analyzer.frame();
            let at = loudest(&bars);
            assert!(at.abs_diff(bar_for(hz)) <= 1, "{hz} Hz lit bar {at}");
            assert!(bars[at] > 0.5, "{hz} Hz only reached {}", bars[at]);
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
        assert!(analyzer.frame().iter().all(|b| (0.0..=1.0).contains(b)));
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
