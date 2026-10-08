//! A spiral galaxy turns behind the logo; inner stars move faster.

use super::palette::{BLUE, CYAN, ICE, MAGENTA, PINK, round, shade};
use super::rng::Rng;
use super::{Effect, Frame};
use std::f32::consts::PI;

struct Star {
    r: f32,
    angle: f32,
    bright: f32,
}

pub struct Galaxy {
    stars: Vec<Star>,
    time: f32,
}

impl Galaxy {
    pub fn new(mut rng: Rng) -> Galaxy {
        let stars = (0..900)
            .map(|_| {
                let arm = rng.below(2) as f32;
                let r = rng.unit().powf(0.8) * 30.0 + 2.0;
                let angle = arm * PI + r * 0.17 + rng.range(-0.3, 0.3) * (1.0 + 4.0 / r);
                Star {
                    r,
                    angle,
                    bright: rng.unit(),
                }
            })
            .collect();
        Galaxy { stars, time: 0.0 }
    }
}

impl Effect for Galaxy {
    fn frame(&mut self, f: &mut Frame) {
        self.time += f.dt * f.boost(3.0);
        let (cx, cy) = f.logo.center();
        let colors = if f.apple() {
            [MAGENTA, BLUE, PINK]
        } else {
            [BLUE, CYAN, ICE]
        };
        for s in &self.stars {
            let a = s.angle + self.time * (0.22 + 0.25 / s.r.sqrt());
            let (x, y) = (
                round(cx + a.cos() * s.r * 1.7),
                round(cy + a.sin() * s.r * 0.45),
            );
            let k = (1.0 - s.r / 34.0).max(0.2) * (0.5 + 0.5 * s.bright);
            let ch = if k > 0.7 {
                '*'
            } else if k > 0.45 {
                '•'
            } else {
                '·'
            };
            let color = colors[if s.bright > 0.85 {
                2
            } else if s.r < 18.0 {
                0
            } else {
                1
            }];
            f.put(x, y, ch, shade(color, 0.3 + 0.7 * k), false);
        }
        f.logo.draw(f.canvas);
    }
}
