//! Fireflies wander slowly, each glowing on its own rhythm.

use super::palette::{BLUE, CYAN, GREEN, LIME, shade};
use super::rng::Rng;
use super::{Effect, Frame, Logo};

const GLOW: [(i32, i32); 6] = [(-1, 0), (1, 0), (0, -1), (0, 1), (-2, 0), (2, 0)];

struct Fly {
    x: f32,
    y: f32,
    vx: f32,
    vy: f32,
    rate: f32,
    phase: f32,
}

pub struct Fireflies {
    rng: Rng,
    flies: Vec<Fly>,
}

impl Fireflies {
    pub fn new(logo: &Logo, mut rng: Rng) -> Fireflies {
        let (w, h) = logo.canvas_size();
        let flies = (0..30)
            .map(|_| Fly {
                x: rng.range(0.0, w as f32),
                y: rng.range(0.0, h as f32),
                vx: 0.0,
                vy: 0.0,
                rate: rng.range(0.4, 1.1),
                phase: rng.range(0.0, 6.0),
            })
            .collect();
        Fireflies { rng, flies }
    }

    fn wander(&mut self, dt: f32, speed: f32, w: f32, h: f32) {
        // A random walk's spread grows with the square root of time, so kicks
        // scale by sqrt(dt); at 60 fps this equals the mockup's per-frame kick.
        let kick = (dt / 60.0).sqrt();
        let drag = (-dt * 0.8).exp();
        for fl in &mut self.flies {
            fl.vx = (fl.vx + self.rng.range(-6.0, 6.0) * kick) * drag;
            fl.vy = (fl.vy + self.rng.range(-3.0, 3.0) * kick) * drag;
            fl.x = (fl.x + fl.vx * dt * 3.0 * speed + w).rem_euclid(w);
            fl.y = (fl.y + fl.vy * dt * 3.0 * speed).clamp(0.0, h - 1.0);
        }
    }
}

impl Effect for Fireflies {
    fn frame(&mut self, f: &mut Frame) {
        let (w, h) = f.size();
        let speed = f.boost(2.0);
        self.wander(f.dt, speed, w as f32, h as f32);
        let (core, glow) = if f.apple() {
            (LIME, GREEN)
        } else {
            (CYAN, BLUE)
        };
        for fl in &self.flies {
            let k = (f.t * fl.rate * speed + fl.phase).sin().max(0.0).powi(3);
            if k < 0.05 {
                continue;
            }
            let (x, y) = (super::palette::round(fl.x), super::palette::round(fl.y));
            if k > 0.5 {
                for (dx, dy) in GLOW {
                    f.put(
                        x + dx,
                        y + dy,
                        '·',
                        shade(glow, 0.5 * k / (dx.abs() + dy.abs()) as f32),
                        false,
                    );
                }
            }
            let ch = if k > 0.6 {
                '●'
            } else if k > 0.25 {
                '•'
            } else {
                '·'
            };
            f.put(x, y, ch, shade(core, 0.3 + 0.7 * k), false);
        }
        f.logo.draw(f.canvas);
    }
}
