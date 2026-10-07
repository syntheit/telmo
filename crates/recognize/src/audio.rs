use crate::signature::SAMPLE_RATE;

/// Downmix interleaved `channels` audio at `rate` Hz to mono 16 kHz.
///
/// Each output sample averages the input span it covers, which doubles as a
/// cheap low-pass filter when downsampling.
pub fn to_16k_mono(samples: &[f32], rate: u32, channels: u16) -> Vec<f32> {
    let channels = usize::from(channels.max(1));
    let mono: Vec<f32> = samples
        .chunks_exact(channels)
        .map(|frame| frame.iter().sum::<f32>() / channels as f32)
        .collect();
    if rate == 0 || rate == SAMPLE_RATE {
        return mono;
    }

    let step = f64::from(rate) / f64::from(SAMPLE_RATE);
    let out_len = (mono.len() as f64 / step) as usize;
    (0..out_len)
        .map(|i| {
            let start = (i as f64 * step) as usize;
            let end = (((i + 1) as f64 * step) as usize).clamp(start + 1, mono.len());
            let span = &mono[start..end];
            span.iter().sum::<f32>() / span.len() as f32
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn same_rate_mono_is_untouched() {
        let input = [0.1, -0.2, 0.3];
        assert_eq!(to_16k_mono(&input, 16_000, 1), input);
    }

    #[test]
    fn stereo_is_averaged() {
        assert_eq!(to_16k_mono(&[1.0, 0.0, 0.5, 0.5], 16_000, 2), [0.5, 0.5]);
    }

    #[test]
    fn downsamples_by_averaging() {
        let input: Vec<f32> = (0..48_000).map(|i| (i % 3) as f32).collect();
        let out = to_16k_mono(&input, 48_000, 1);
        assert_eq!(out.len(), 16_000);
        assert!(out.iter().all(|s| (s - 1.0).abs() < 1e-6));
    }

    #[test]
    fn upsamples_length() {
        let out = to_16k_mono(&vec![0.5; 8_000], 8_000, 1);
        assert_eq!(out.len(), 16_000);
    }

    #[test]
    fn ragged_input_does_not_panic() {
        assert!(to_16k_mono(&[0.5; 7], 44_100, 2).len() <= 1);
        assert!(to_16k_mono(&[], 44_100, 2).is_empty());
    }
}
