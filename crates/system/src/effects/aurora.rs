//! Northern lights: slow curtains of light under a few faint stars.

use super::palette::{BLUE, CYAN, GREEN, MAGENTA, WHITE, shade};
use super::rng::Rng;
use super::{Effect, Frame, Logo};

struct Star {
    x: i32,
    y: i32,
    phase: f32,
}

pub struct Aurora {
    stars: Vec<Star>,
    time: f32,
}

impl Aurora {
    pub fn new(logo: &Logo, mut rng: Rng) -> Aurora {
        let (w, h) = logo.canvas_size();
        let stars = (0..40)
            .map(|_| Star {
                x: rng.below(w as usize) as i32,
                y: rng.below(h as usize) as i32,
                phase: rng.range(0.0, 6.0),
            })
            .collect();
        Aurora { stars, time: 0.0 }
    }
}

fn curtain(f: &mut Frame, color: ratatui::style::Color, off: f32, t: f32) {
    let (w, h) = f.size();
    for x in 0..w {
        let xf = x as f32;
        let top = 2.0
            + off
            + 2.6 * (xf * 0.06 + t * 0.35 + off).sin()
            + 1.6 * (xf * 0.15 - t * 0.6 + off * 2.0).sin();
        let len = 7.0 + 3.0 * (xf * 0.045 + t * 0.25 + off).sin();
        let rays = 0.55 + 0.45 * (xf * 1.3 + 3.0 * (xf * 0.2 + t * 0.8).sin() + off).sin();
        let shimmer = rays * (0.6 + 0.4 * (xf * 0.11 - t * 0.9 + off).sin());
        let end = (h as f32).min(top + len);
        let mut y = (top.floor() as i32).max(0);
        while (y as f32) < end {
            let d = y as f32 - top;
            let k = shimmer * (1.0 - d / len).powf(1.4) * if d < 1.0 { d } else { 1.0 };
            if k >= 0.08 {
                let ch = if k > 0.65 {
                    '▓'
                } else if k > 0.4 {
                    '▒'
                } else if k > 0.2 {
                    '░'
                } else {
                    '·'
                };
                f.put(x, y, ch, shade(color, 0.25 + 0.75 * k), false);
            }
            y += 1;
        }
    }
}

impl Effect for Aurora {
    fn frame(&mut self, f: &mut Frame) {
        self.time += f.dt * f.boost(2.2);
        for s in &self.stars {
            f.put(
                s.x,
                s.y,
                '·',
                shade(WHITE, 0.25 + 0.2 * (f.t * 1.5 + s.phase).sin()),
                false,
            );
        }
        let layers = if f.apple() {
            [(GREEN, 0.0), (MAGENTA, 2.4)]
        } else {
            [(CYAN, 0.0), (BLUE, 2.4)]
        };
        for (color, off) in layers {
            curtain(f, color, off, self.time);
        }
        f.logo.draw(f.canvas);
    }
}
