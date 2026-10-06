//! Pure math: latency statistics and throughput from byte-counter samples.

/// `(seconds since start, total bytes)`, taken every 100 ms.
pub type Sample = (f64, u64);

/// A phase lasts this long unless the speed is still climbing.
pub const BASE_SECS: f64 = 10.0;
pub const MAX_SECS: f64 = 15.0;
/// Everything before this is TCP slow start and socket buffers.
const WARMUP_SECS: f64 = 3.0;
const WINDOW_SECS: f64 = 1.0;

pub fn median(values: &[f64]) -> f64 {
    if values.is_empty() {
        return 0.0;
    }
    let mut sorted = values.to_vec();
    sorted.sort_by(f64::total_cmp);
    let mid = sorted.len() / 2;
    if sorted.len().is_multiple_of(2) {
        (sorted[mid - 1] + sorted[mid]) / 2.0
    } else {
        sorted[mid]
    }
}

/// Mean absolute difference of consecutive samples.
pub fn jitter(values: &[f64]) -> f64 {
    if values.len() < 2 {
        return 0.0;
    }
    let sum: f64 = values.windows(2).map(|w| (w[1] - w[0]).abs()).sum();
    sum / (values.len() - 1) as f64
}

/// Last sample taken at or before `t` (the first one if none is).
fn at(samples: &[Sample], t: f64) -> Sample {
    let i = samples.partition_point(|s| s.0 <= t);
    samples[i.saturating_sub(1)]
}

/// Average Mbit/s between two times.
fn rate(samples: &[Sample], from: f64, to: f64) -> f64 {
    let (t0, b0) = at(samples, from);
    let (t1, b1) = at(samples, to);
    if t1 <= t0 {
        return 0.0;
    }
    (b1 - b0) as f64 * 8.0 / (t1 - t0) / 1e6
}

/// Live speed: the last second.
pub fn live_mbps(samples: &[Sample]) -> f64 {
    let now = samples.last().map_or(0.0, |s| s.0);
    rate(samples, now - WINDOW_SECS, now)
}

/// The phase runs 10 s; if the speed is still climbing then (the last 2 s
/// are over 1.2x the 3-8 s average) it runs up to 15 s.
pub fn needs_extension(samples: &[Sample]) -> bool {
    let steady = rate(samples, WARMUP_SECS, 8.0);
    rate(samples, BASE_SECS - 2.0, BASE_SECS) > steady * 1.2
}

/// Steady-state average, excluding warm-up. A 10 s phase uses 3-10 s, an
/// extended one its last 7 s.
pub fn final_mbps(samples: &[Sample]) -> f64 {
    let end = samples.last().map_or(0.0, |s| s.0);
    let from = if end > BASE_SECS + 0.5 {
        end - 7.0
    } else {
        WARMUP_SECS
    };
    rate(samples, from, end)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Constant `mbps` for `secs`, sampled every 100 ms.
    fn constant(mbps: f64, secs: u32) -> Vec<Sample> {
        (0..=secs * 10)
            .map(|i| {
                let t = f64::from(i) / 10.0;
                (t, (mbps * 1e6 / 8.0 * t) as u64)
            })
            .collect()
    }

    #[test]
    fn median_odd_even_empty() {
        assert_eq!(median(&[3.0, 1.0, 2.0]), 2.0);
        assert_eq!(median(&[4.0, 1.0, 2.0, 3.0]), 2.5);
        assert_eq!(median(&[]), 0.0);
    }

    #[test]
    fn jitter_is_mean_abs_diff() {
        assert_eq!(jitter(&[10.0, 12.0, 11.0, 15.0]), (2.0 + 1.0 + 4.0) / 3.0);
        assert_eq!(jitter(&[5.0]), 0.0);
    }

    #[test]
    fn steady_rate_of_constant_stream() {
        let samples = constant(100.0, 10);
        assert!((final_mbps(&samples) - 100.0).abs() < 0.5);
        assert!((live_mbps(&samples) - 100.0).abs() < 0.5);
        assert!(!needs_extension(&samples));
    }

    #[test]
    fn warmup_is_excluded() {
        // Nothing for 3 s, then 80 Mbit/s: the average starts at 3 s.
        let samples: Vec<Sample> = constant(80.0, 10)
            .into_iter()
            .map(|(t, b)| (t, if t < 3.0 { 0 } else { b - 30_000_000 }))
            .collect();
        assert!((final_mbps(&samples) - 80.0).abs() < 0.5);
    }

    #[test]
    fn climbing_speed_extends_and_uses_last_7s() {
        // 50 Mbit/s until 8 s, then 100.
        let mut samples = constant(50.0, 8);
        let base = samples.last().unwrap().1;
        for i in 81..=150 {
            let t = f64::from(i) / 10.0;
            samples.push((t, base + (100e6 / 8.0 * (t - 8.0)) as u64));
        }
        assert!(needs_extension(&samples[..=100]));
        assert!((final_mbps(&samples) - 100.0).abs() < 0.5);
    }

    #[test]
    fn no_samples_is_zero() {
        assert_eq!(final_mbps(&[(0.0, 0)]), 0.0);
        assert_eq!(live_mbps(&[(0.0, 0)]), 0.0);
    }
}
