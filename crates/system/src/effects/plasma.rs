//! The logo's own characters recolored by slow sine waves.

use super::palette::hsl;
use super::{Effect, Frame};

pub struct Plasma {
    time: f32,
}

impl Plasma {
    pub fn new() -> Plasma {
        Plasma { time: 0.0 }
    }
}

impl Effect for Plasma {
    fn frame(&mut self, f: &mut Frame) {
        self.time += f.dt * f.boost(2.5);
        let t = self.time;
        let apple = f.apple();
        f.logo.draw_with(f.canvas, |c| {
            let (lx, ly) = (c.lx as f32, c.ly as f32);
            let v = (lx * 0.16 + t * 1.1).sin()
                + (ly * 0.35 - t * 0.9).sin()
                + ((lx * 0.5 + ly) * 0.12 + t * 0.6).sin();
            let hue = if apple {
                (v * 50.0 + t * 30.0 + 360.0).rem_euclid(360.0)
            } else {
                210.0 + 28.0 * (v * 0.9 + t * 0.5).sin()
            };
            hsl(hue.round(), 0.82, 0.70)
        });
    }
}
