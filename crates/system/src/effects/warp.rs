//! Stars stream out from behind the logo and stretch into streaks.

use super::palette::{BLUE, CYAN, WHITE, rainbow, round, shade};
use super::rng::Rng;
use super::{Effect, Frame};
use std::f32::consts::TAU;

const MAX_STARS: usize = 110;

struct Star {
    angle: f32,
    r: f32,
    v: f32,
    hue: f32,
}

pub struct Warp {
    rng: Rng,
    stars: Vec<Star>,
}

impl Warp {
    pub fn new(rng: Rng) -> Warp {
        Warp {
            rng,
            stars: Vec::new(),
        }
    }

    fn spawn(&mut self, dt: f32) {
        // Three a frame at 60 fps.
        for _ in 0..self.rng.count(180.0 * dt) {
            if self.stars.len() < MAX_STARS {
                let star = Star {
                    angle: self.rng.range(0.0, TAU),
                    r: self.rng.range(1.0, 20.0),
                    v: self.rng.range(2.0, 5.0),
                    hue: self.rng.unit(),
                };
                self.stars.push(star);
            }
        }
    }

    fn advance(&mut self, dt: f32, speed: f32) {
        for s in &mut self.stars {
            s.v *= (1.5 * dt * speed).exp();
            s.r += s.v * dt * speed;
        }
        self.stars.retain(|s| s.r < 80.0);
    }
}

fn streak(cos: f32, sin: f32) -> char {
    if cos.abs() > 0.88 {
        '─'
    } else if sin.abs() > 0.93 {
        '│'
    } else if cos * sin > 0.0 {
        '╲'
    } else {
        '╱'
    }
}

fn draw_star(f: &mut Frame, s: &Star, center: (f32, f32)) {
    let col = if f.apple() {
        rainbow()[(s.hue * 6.0) as usize]
    } else if s.hue < 0.5 {
        CYAN
    } else {
        BLUE
    };
    let tail = (s.v * 0.12).min(7.0);
    let (cos, sin) = (s.angle.cos(), s.angle.sin());
    let line = streak(cos, sin);
    let mut k = tail;
    while k >= 0.0 {
        let r = s.r - k;
        let (x, y) = (round(center.0 + cos * r), round(center.1 + sin * r * 0.5));
        let head = k < 0.5;
        let fade = if head {
            1.0
        } else {
            0.35 + 0.65 * (1.0 - k / tail.max(0.01))
        };
        let bright = (0.2 + s.r / 40.0).min(1.0) * fade;
        let ch = match (head, s.v) {
            (true, v) if v > 18.0 => line,
            (true, v) if v > 8.0 => '•',
            (true, _) => '·',
            _ if tail > 1.5 => line,
            _ => '·',
        };
        let color = if head && s.r > 30.0 {
            shade(WHITE, bright)
        } else {
            shade(col, bright)
        };
        f.put(x, y, ch, color, head);
        k -= 0.5;
    }
}

impl Effect for Warp {
    fn frame(&mut self, f: &mut Frame) {
        let speed = f.boost(2.4);
        self.spawn(f.dt);
        self.advance(f.dt, speed);
        let center = f.logo.center();
        for s in &self.stars {
            draw_star(f, s, center);
        }
        f.logo.draw(f.canvas);
    }
}
