//! Shazam audio signatures: spectral peak picking and the binary container.
//!
//! Port of SongRec's `fingerprinting/algorithm.rs` and `signature_format.rs`.

use rustfft::{FftPlanner, num_complex::Complex};

pub const SAMPLE_RATE: u32 = 16_000;

const WINDOW: usize = 2048;
const BINS: usize = 1025;
const HOP: usize = 128;
const HISTORY: usize = 256;
const PEAK_DELAY: u32 = 46;
const SIGNATURE_SECONDS: usize = 12;

const MAGIC_1: u32 = 0xcafe_2580;
const MAGIC_2: u32 = 0x9411_9c00;
const SAMPLE_RATE_ID_16K: u32 = 3;
const BAND_TAG: u32 = 0x6003_0040;
const HEADER_LEN: usize = 48;

type Spectrum = [f32; BINS];

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Peak {
    pub pass: u32,
    pub magnitude: u16,
    /// Frequency bin scaled by 64 (sub-bin precision).
    pub bin: u16,
}

/// Peaks grouped by band: 250-520, 520-1450, 1450-3500, 3500-5500 Hz.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Signature {
    pub number_samples: u32,
    pub bands: [Vec<Peak>; 4],
}

/// Fingerprint up to 12 s (from the middle) of 16 kHz mono audio.
pub fn signature(samples_16k_mono: &[f32]) -> Signature {
    let want = SIGNATURE_SECONDS * SAMPLE_RATE as usize;
    let mut samples = samples_16k_mono.to_vec();
    if samples.len() > want {
        let start = (samples.len() - want) / 2;
        samples = samples[start..start + want].to_vec();
    } else {
        samples.resize(want, 0.0);
    }
    Fingerprinter::new().run(&samples)
}

fn hann(i: usize) -> f32 {
    // SongRec's table: a Hann window with the zero endpoints dropped.
    // sin^2 form of 0.5 - 0.5cos, which stays precise near the edges.
    (std::f32::consts::PI * (i + 1) as f32 / (WINDOW + 1) as f32)
        .sin()
        .powi(2)
}

struct Fingerprinter {
    fft: std::sync::Arc<dyn rustfft::Fft<f32>>,
    window: Vec<f32>,
    samples: [i16; WINDOW],
    spectra: Vec<Spectrum>,
    spread: Vec<Spectrum>,
    /// Both rings are indexed by a wrapping u8, hence 256 entries.
    next: u8,
    done: u32,
    out: Signature,
}

impl Fingerprinter {
    fn new() -> Self {
        Self {
            fft: FftPlanner::new().plan_fft_forward(WINDOW),
            window: (0..WINDOW).map(hann).collect(),
            samples: [0; WINDOW],
            spectra: vec![[0.0; BINS]; HISTORY],
            spread: vec![[0.0; BINS]; HISTORY],
            next: 0,
            done: 0,
            out: Signature::default(),
        }
    }

    fn run(mut self, samples: &[f32]) -> Signature {
        self.out.number_samples = samples.len() as u32;
        let pcm: Vec<i16> = samples
            .iter()
            .map(|s| (s * 32768.0).clamp(-32768.0, 32767.0) as i16)
            .collect();
        for (n, chunk) in pcm.as_chunks::<HOP>().0.iter().enumerate() {
            // Ring position of the oldest sample in the window.
            let oldest = ((n + 1) * HOP) % WINDOW;
            self.samples[(n * HOP) % WINDOW..][..HOP].copy_from_slice(chunk);
            self.push_spectrum(oldest);
            self.spread_latest();
            self.next = self.next.wrapping_add(1);
            self.done += 1;
            if self.done >= PEAK_DELAY {
                self.pick_peaks();
            }
        }
        self.out
    }

    fn push_spectrum(&mut self, oldest: usize) {
        let mut buf: Vec<Complex<f32>> = (0..WINDOW)
            .map(|i| {
                let s = f32::from(self.samples[(i + oldest) % WINDOW]);
                Complex::new(s * self.window[i], 0.0)
            })
            .collect();
        self.fft.process(&mut buf);
        let out = &mut self.spectra[usize::from(self.next)];
        for (o, c) in out.iter_mut().zip(&buf) {
            *o = (c.norm_sqr() / (1u32 << 17) as f32).max(1e-10);
        }
    }

    /// Smear the newest spectrum across neighbouring bins, then back into the
    /// spread spectra of the frames 1, 3 and 6 steps earlier.
    fn spread_latest(&mut self) {
        let now = usize::from(self.next);
        let mut row = self.spectra[now];
        for i in 0..BINS - 2 {
            row[i] = row[i].max(row[i + 1]).max(row[i + 2]);
        }
        self.spread[now] = row;
        for back in [1u8, 3, 6] {
            let old = &mut self.spread[usize::from(self.next.wrapping_sub(back))];
            for (o, r) in old.iter_mut().zip(&row) {
                *o = o.max(*r);
            }
        }
    }

    fn pick_peaks(&mut self) {
        // `next` already points past the newest frame.
        let spectrum = self.spectra[usize::from(self.next.wrapping_sub(PEAK_DELAY as u8))];
        let spread = &self.spread[usize::from(self.next.wrapping_sub(49))];
        let pass = self.done - PEAK_DELAY;

        for bin in 10..=1014 {
            let level = spectrum[bin];
            if level < 1.0 / 64.0 || level < spread[bin - 1] {
                continue;
            }
            let mut neighbours = [-10isize, -7, -4, -3, 1, 2, 5, 8]
                .iter()
                .map(|o| spread[(bin as isize + o) as usize])
                .fold(0.0, f32::max);
            if level <= neighbours {
                continue;
            }
            for offset in [
                -53isize, -45, 165, 172, 179, 186, 193, 200, 214, 221, 228, 235, 242, 249,
            ] {
                let frame = (isize::from(self.next) + offset).rem_euclid(HISTORY as isize);
                neighbours = neighbours.max(self.spread[frame as usize][bin - 1]);
            }
            if level <= neighbours {
                continue;
            }
            if let Some(peak) = refine(&spectrum, bin, pass) {
                let hz = f32::from(peak.bin) * (SAMPLE_RATE as f32 / 2.0 / 1024.0 / 64.0);
                if let Some(band) = band_of(hz) {
                    self.out.bands[band].push(peak);
                }
            }
        }
    }
}

fn magnitude(level: f32) -> f32 {
    level.ln().max(1.0 / 64.0) * 1477.3 + 6144.0
}

/// Interpolate the true peak position between bins.
fn refine(spectrum: &Spectrum, bin: usize, pass: u32) -> Option<Peak> {
    let (mid, before, after) = (
        magnitude(spectrum[bin]),
        magnitude(spectrum[bin - 1]),
        magnitude(spectrum[bin + 1]),
    );
    let curvature = mid * 2.0 - before - after;
    if curvature < 0.0 {
        return None;
    }
    let shift = (after - before) * 32.0 / curvature;
    Some(Peak {
        pass,
        magnitude: mid as u16,
        bin: (bin as i32 * 64 + shift as i32) as u16,
    })
}

fn band_of(hz: f32) -> Option<usize> {
    match hz as i32 {
        250..=519 => Some(0),
        520..=1449 => Some(1),
        1450..=3499 => Some(2),
        3500..=5500 => Some(3),
        _ => None,
    }
}

impl Signature {
    pub fn is_empty(&self) -> bool {
        self.bands.iter().all(Vec::is_empty)
    }

    /// Length of the fingerprinted audio in milliseconds.
    pub fn duration_ms(&self) -> u32 {
        (u64::from(self.number_samples) * 1000 / u64::from(SAMPLE_RATE)) as u32
    }

    /// Shazam's binary signature container (little-endian throughout).
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut body = Vec::new();
        for (band, peaks) in self.bands.iter().enumerate() {
            if peaks.is_empty() {
                continue;
            }
            let data = encode_peaks(peaks);
            push(&mut body, BAND_TAG + band as u32);
            push(&mut body, data.len() as u32);
            body.extend_from_slice(&data);
            body.resize(body.len().next_multiple_of(4), 0);
        }
        let size = (body.len() + 8) as u32;
        let samples = self.number_samples + (SAMPLE_RATE as f32 * 0.24) as u32;

        let mut out = Vec::new();
        for word in [
            MAGIC_1,
            0, // crc32, filled below
            size,
            MAGIC_2,
            0,
            0,
            0,
            SAMPLE_RATE_ID_16K << 27,
            0,
            0,
            samples,
            (15 << 19) + 0x40000,
            0x4000_0000,
            size,
        ] {
            push(&mut out, word);
        }
        out.extend_from_slice(&body);
        let crc = crc32(&out[8..]);
        out[4..8].copy_from_slice(&crc.to_le_bytes());
        out
    }

    /// `data:` URI form used in the recognition request.
    pub fn to_uri(&self) -> String {
        use base64::{Engine, prelude::BASE64_STANDARD};
        format!(
            "data:audio/vnd.shazam.sig;base64,{}",
            BASE64_STANDARD.encode(self.to_bytes())
        )
    }

    /// Parse the binary container; `None` if it is malformed.
    pub fn from_bytes(data: &[u8]) -> Option<Self> {
        if data.len() < HEADER_LEN + 8
            || word(data, 0)? != MAGIC_1
            || word(data, 12)? != MAGIC_2
            || word(data, 8)? as usize != data.len() - HEADER_LEN
            || word(data, 4)? != crc32(&data[8..])
            || word(data, 28)? >> 27 != SAMPLE_RATE_ID_16K
        {
            return None;
        }
        let number_samples = word(data, 40)?.checked_sub((SAMPLE_RATE as f32 * 0.24) as u32)?;
        let mut sig = Signature {
            number_samples,
            ..Self::default()
        };

        let mut at = HEADER_LEN + 8;
        while at < data.len() {
            let band = word(data, at)?.checked_sub(BAND_TAG)? as usize;
            let len = word(data, at + 4)? as usize;
            let peaks = data.get(at + 8..at + 8 + len)?;
            *sig.bands.get_mut(band)? = decode_peaks(peaks)?;
            at += 8 + len.next_multiple_of(4);
        }
        Some(sig)
    }
}

fn push(out: &mut Vec<u8>, word: u32) {
    out.extend_from_slice(&word.to_le_bytes());
}

fn word(data: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_le_bytes(data.get(at..at + 4)?.try_into().ok()?))
}

/// Peaks are delta-coded by pass number; a 0xff marker carries an absolute one.
fn encode_peaks(peaks: &[Peak]) -> Vec<u8> {
    let mut out = Vec::new();
    let mut pass = 0;
    for peak in peaks {
        if peak.pass - pass >= 255 {
            out.push(0xff);
            out.extend_from_slice(&peak.pass.to_le_bytes());
            pass = peak.pass;
        }
        out.push((peak.pass - pass) as u8);
        out.extend_from_slice(&peak.magnitude.to_le_bytes());
        out.extend_from_slice(&peak.bin.to_le_bytes());
        pass = peak.pass;
    }
    out
}

fn decode_peaks(data: &[u8]) -> Option<Vec<Peak>> {
    let mut peaks = Vec::new();
    let mut pass = 0u32;
    let mut at = 0;
    while at < data.len() {
        let delta = *data.get(at)?;
        at += 1;
        if delta == 0xff {
            pass = u32::from_le_bytes(data.get(at..at + 4)?.try_into().ok()?);
            at += 4;
            continue;
        }
        let rest = data.get(at..at + 4)?;
        pass += u32::from(delta);
        peaks.push(Peak {
            pass,
            magnitude: u16::from_le_bytes([rest[0], rest[1]]),
            bin: u16::from_le_bytes([rest[2], rest[3]]),
        });
        at += 4;
    }
    Some(peaks)
}

fn crc32(data: &[u8]) -> u32 {
    let mut crc = !0u32;
    for byte in data {
        crc ^= u32::from(*byte);
        for _ in 0..8 {
            crc = (crc >> 1) ^ (0xedb8_8320 & (crc & 1).wrapping_neg());
        }
    }
    !crc
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sine(hz: f32, seconds: usize) -> Vec<f32> {
        (0..SAMPLE_RATE as usize * seconds)
            .map(|i| (std::f32::consts::TAU * hz * i as f32 / SAMPLE_RATE as f32).sin() * 0.5)
            .collect()
    }

    #[test]
    fn crc32_known_vector() {
        assert_eq!(crc32(b"123456789"), 0xcbf4_3926);
    }

    #[test]
    fn hann_matches_songrec_table() {
        assert!((hann(0) - 0.000_002_350_8).abs() < 1e-9);
        assert!((hann(1) - 0.000_009_403_2).abs() < 1e-9);
        assert!((hann(2047) - 0.000_002_350_8).abs() < 1e-9);
        assert!((hann(1023) - 1.0).abs() < 1e-5);
    }

    #[test]
    fn bands_split_at_documented_edges() {
        assert_eq!(band_of(249.9), None);
        assert_eq!(band_of(250.0), Some(0));
        assert_eq!(band_of(520.0), Some(1));
        assert_eq!(band_of(1450.0), Some(2));
        assert_eq!(band_of(3500.0), Some(3));
        assert_eq!(band_of(5501.0), None);
    }

    #[test]
    fn peak_encoding_roundtrips_across_large_gaps() {
        let peaks = vec![
            Peak {
                pass: 3,
                magnitude: 7000,
                bin: 12_345,
            },
            Peak {
                pass: 3,
                magnitude: 6500,
                bin: 20_000,
            },
            Peak {
                pass: 1000,
                magnitude: 6200,
                bin: 400,
            },
            Peak {
                pass: 1100,
                magnitude: 6100,
                bin: 65_000,
            },
        ];
        assert_eq!(decode_peaks(&encode_peaks(&peaks)), Some(peaks));
    }

    #[test]
    fn peak_delta_bytes_are_as_documented() {
        let bytes = encode_peaks(&[Peak {
            pass: 2,
            magnitude: 0x0102,
            bin: 0x0304,
        }]);
        assert_eq!(bytes, [2, 0x02, 0x01, 0x04, 0x03]);
    }

    #[test]
    fn signature_roundtrips_through_bytes() {
        let sig = signature(&sine(1000.0, 12));
        assert!(!sig.is_empty());
        assert_eq!(Signature::from_bytes(&sig.to_bytes()), Some(sig));
    }

    #[test]
    fn header_layout() {
        let bytes = Signature::default().to_bytes();
        assert_eq!(&bytes[..4], [0x80, 0x25, 0xfe, 0xca]);
        assert_eq!(&bytes[12..16], [0x00, 0x9c, 0x11, 0x94]);
        assert_eq!(bytes.len(), 56);
        assert_eq!(&bytes[28..32], [0, 0, 0, 0x18]);
    }

    #[test]
    fn corrupt_input_is_rejected() {
        let mut bytes = signature(&sine(1000.0, 12)).to_bytes();
        let last = bytes.len() - 1;
        bytes[last] ^= 1;
        assert_eq!(Signature::from_bytes(&bytes), None);
        assert_eq!(Signature::from_bytes(&[0; 10]), None);
    }

    #[test]
    fn sine_peaks_land_in_the_right_band() {
        let sig = signature(&sine(1000.0, 12));
        assert!(sig.bands[1].len() > 100);
        // Quantisation noise adds weak extra peaks; the loudest is the tone.
        let loudest = sig
            .bands
            .iter()
            .flatten()
            .max_by_key(|p| p.magnitude)
            .unwrap();
        let hz = f32::from(loudest.bin) * (8000.0 / 1024.0 / 64.0);
        assert!((hz - 1000.0).abs() < 10.0, "{hz}");
    }

    #[test]
    fn silence_has_no_peaks() {
        assert!(signature(&[0.0; 16_000]).is_empty());
    }

    #[test]
    fn long_input_uses_twelve_seconds() {
        let sig = signature(&sine(440.0, 20));
        assert_eq!(sig.duration_ms(), 12_000);
    }
}
