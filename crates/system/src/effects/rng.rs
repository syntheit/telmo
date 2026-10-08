//! A tiny xorshift64* generator; the effects only need cheap, seedable noise.

use std::time::{SystemTime, UNIX_EPOCH};

pub fn clock_seed() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(1)
}

#[derive(Clone)]
pub struct Rng(u64);

impl Rng {
    pub fn new(seed: u64) -> Rng {
        // Scramble the seed so small seeds (0, 1, 2) give unrelated streams; never zero.
        Rng(seed
            .wrapping_add(0x9E37_79B9_7F4A_7C15)
            .wrapping_mul(0xBF58_476D_1CE4_E5B9)
            | 1)
    }

    pub fn from_clock() -> Rng {
        Rng::new(clock_seed())
    }

    fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    /// Uniform in [0, 1).
    pub fn unit(&mut self) -> f32 {
        (self.next() >> 40) as f32 / (1u64 << 24) as f32
    }

    pub fn range(&mut self, a: f32, b: f32) -> f32 {
        a + self.unit() * (b - a)
    }

    pub fn chance(&mut self, p: f32) -> bool {
        self.unit() < p
    }

    /// Uniform in 0..n (0 when n is 0).
    pub fn below(&mut self, n: usize) -> usize {
        ((self.unit() * n as f32) as usize).min(n.saturating_sub(1))
    }

    pub fn pick<T: Copy>(&mut self, items: &[T]) -> T {
        items[self.below(items.len())]
    }

    /// A whole number whose average is `x`: how many events a fractional rate
    /// produces in one frame.
    pub fn count(&mut self, x: f32) -> usize {
        let whole = x.floor();
        whole as usize + usize::from(self.chance(x - whole))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn values_stay_in_range() {
        let mut rng = Rng::new(0);
        for _ in 0..10_000 {
            let v = rng.range(-2.0, 3.0);
            assert!((-2.0..3.0).contains(&v));
            assert!(rng.below(7) < 7);
        }
    }

    #[test]
    fn count_averages_to_its_argument() {
        let mut rng = Rng::new(3);
        let total: usize = (0..20_000).map(|_| rng.count(0.4)).sum();
        assert!((7_000..9_000).contains(&total), "{total}");
    }
}
