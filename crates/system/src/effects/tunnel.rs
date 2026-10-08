//! The demoscene tunnel: a twisting checkerboard tube rushing toward you.

use super::palette::{BLUE, CYAN, rainbow, shade};
use super::{Effect, Frame};
use std::f32::consts::PI;

pub struct Tunnel {
    time: f32,
}

impl Tunnel {
    pub fn new() -> Tunnel {
        Tunnel { time: 0.0 }
    }
}

impl Effect for Tunnel {
    fn frame(&mut self, f: &mut Frame) {
        self.time += f.dt * f.boost(2.4);
        let t = self.time;
        let (w, h) = f.size();
        let (cx, cy) = f.logo.center();
        let bands = if f.apple() {
            rainbow().to_vec()
        } else {
            vec![BLUE, CYAN]
        };
        for y in 0..h {
            for x in 0..w {
                if f.logo.masked(x, y) {
                    continue;
                }
                let dx = x as f32 - cx;
                let dy = (y as f32 - cy) * 2.1;
                let r = dx.hypot(dy) + 0.5;
                let depth = 48.0 / r + t * 2.2;
                let angle = (dy.atan2(dx) / PI) * 6.0 + t * 0.35 + 0.8 * (depth * 0.3).sin();
                if (depth.floor() as i32 + angle.floor() as i32) & 1 == 0 {
                    continue;
                }
                let light = (r / 34.0).min(1.0);
                let color = bands[(depth.floor() as usize) % bands.len()];
                let ch = if light > 0.55 {
                    '▒'
                } else if light > 0.3 {
                    '░'
                } else {
                    '·'
                };
                f.canvas
                    .set(x, y, ch, shade(color, 0.15 + 0.75 * light), false);
            }
        }
        f.logo.draw(f.canvas);
    }
}
