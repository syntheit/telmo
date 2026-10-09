//! The frequency response of a preset, for drawing its curve. The same RBJ
//! cookbook biquads the audio engine in Telmo.app runs.

use crate::eq::{Filter, Shape};
use std::f64::consts::PI;

/// The curve is drawn for this sample rate; the shapes barely move with it.
const RATE: f64 = 48_000.0;

/// `[b0, b1, b2, a0, a1, a2]` of one filter.
fn coefficients(filter: &Filter) -> [f64; 6] {
    let a = 10f64.powf(filter.gain / 40.0);
    let w0 = 2.0 * PI * filter.freq / RATE;
    let (cos, sin) = (w0.cos(), w0.sin());
    let alpha = sin / (2.0 * filter.q);
    let s = 2.0 * a.sqrt() * alpha;
    match filter.shape {
        Shape::Peak => [
            1.0 + alpha * a,
            -2.0 * cos,
            1.0 - alpha * a,
            1.0 + alpha / a,
            -2.0 * cos,
            1.0 - alpha / a,
        ],
        Shape::LowShelf => [
            a * ((a + 1.0) - (a - 1.0) * cos + s),
            2.0 * a * ((a - 1.0) - (a + 1.0) * cos),
            a * ((a + 1.0) - (a - 1.0) * cos - s),
            (a + 1.0) + (a - 1.0) * cos + s,
            -2.0 * ((a - 1.0) + (a + 1.0) * cos),
            (a + 1.0) + (a - 1.0) * cos - s,
        ],
        Shape::HighShelf => [
            a * ((a + 1.0) + (a - 1.0) * cos + s),
            -2.0 * a * ((a - 1.0) + (a + 1.0) * cos),
            a * ((a + 1.0) + (a - 1.0) * cos - s),
            (a + 1.0) - (a - 1.0) * cos + s,
            2.0 * ((a - 1.0) - (a + 1.0) * cos),
            (a + 1.0) - (a - 1.0) * cos - s,
        ],
        Shape::HighPass => [
            (1.0 + cos) / 2.0,
            -(1.0 + cos),
            (1.0 + cos) / 2.0,
            1.0 + alpha,
            -2.0 * cos,
            1.0 - alpha,
        ],
    }
}

/// The summed gain in dB of all `filters` at `hz`.
pub fn gain_db(filters: &[Filter], hz: f64) -> f64 {
    let w = 2.0 * PI * hz / RATE;
    let (z1, z2) = ((w.cos(), -w.sin()), ((2.0 * w).cos(), -(2.0 * w).sin()));
    let at = |c0: f64, c1: f64, c2: f64| (c0 + c1 * z1.0 + c2 * z2.0, c1 * z1.1 + c2 * z2.1);
    filters
        .iter()
        .map(|filter| {
            let [b0, b1, b2, a0, a1, a2] = coefficients(filter);
            let (nr, ni) = at(b0, b1, b2);
            let (dr, di) = at(a0, a1, a2);
            10.0 * ((nr * nr + ni * ni) / (dr * dr + di * di)).log10()
        })
        .sum()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn filter(shape: Shape, freq: f64, gain: f64, q: f64) -> Filter {
        Filter {
            shape,
            freq,
            gain,
            q,
        }
    }

    #[test]
    fn a_peak_has_its_gain_at_its_center() {
        let peak = [filter(Shape::Peak, 160.0, 3.0, 0.7)];
        assert!((gain_db(&peak, 160.0) - 3.0).abs() < 1e-6);
        assert!(gain_db(&peak, 10_000.0).abs() < 0.1);
    }

    #[test]
    fn a_shelf_reaches_its_gain_far_from_its_corner() {
        let low = [filter(Shape::LowShelf, 100.0, 4.0, 0.7)];
        assert!((gain_db(&low, 20.0) - 4.0).abs() < 0.3);
        assert!(gain_db(&low, 10_000.0).abs() < 0.1);
        let high = [filter(Shape::HighShelf, 8000.0, -2.0, 0.7)];
        assert!((gain_db(&high, 20_000.0) + 2.0).abs() < 0.5);
    }

    #[test]
    fn a_high_pass_cuts_below_and_passes_above() {
        let cut = [filter(Shape::HighPass, 40.0, 0.0, 0.7)];
        assert!(gain_db(&cut, 20.0) < -6.0);
        assert!(gain_db(&cut, 1000.0).abs() < 0.01);
    }

    #[test]
    fn filters_add_up() {
        let a = filter(Shape::Peak, 100.0, 3.0, 1.0);
        let b = filter(Shape::Peak, 2000.0, -2.0, 1.0);
        let both = gain_db(&[a, b], 100.0);
        assert!((both - (gain_db(&[a], 100.0) + gain_db(&[b], 100.0))).abs() < 1e-9);
        assert_eq!(gain_db(&[], 100.0), 0.0);
    }

    #[test]
    fn the_earfun_replica_peaks_where_the_research_says() {
        let earfun = crate::eq::Kind::EarFun.presets()[1];
        let db = gain_db(earfun.filters, 63.0);
        assert!((db - 9.6).abs() < 0.2, "{db}");
    }
}
