//! A radar sweep with a phosphor trail and contacts that light up.

use super::palette::{CYAN, FAINT, GREEN, WHITE, YELLOW, shade};
use super::rng::Rng;
use super::{Effect, Frame, Logo};
use std::f32::consts::TAU;

/// Cells are about twice as tall as wide.
const ASPECT: f32 = 2.1;
const TRAIL: f32 = 1.25;

struct Blip {
    x: i32,
    y: i32,
    lit: f32,
}

pub struct Radar {
    rng: Rng,
    angle: f32,
    blips: Vec<Blip>,
}

impl Radar {
    pub fn new(logo: &Logo, rng: Rng) -> Radar {
        let mut radar = Radar {
            rng,
            angle: 0.0,
            blips: Vec::new(),
        };
        radar.blips = (0..8).map(|_| radar.blip(logo)).collect();
        radar
    }

    fn blip(&mut self, logo: &Logo) -> Blip {
        let (w, h) = logo.canvas_size();
        let mut blip = Blip {
            x: 0,
            y: 0,
            lit: -99.0,
        };
        for _ in 0..200 {
            blip.x = self.rng.range(2.0, (w - 2) as f32).floor() as i32;
            blip.y = self.rng.range(1.0, h as f32 - 1.0).floor() as i32;
            if !logo.masked(blip.x, blip.y) {
                break;
            }
        }
        blip
    }

    fn draw_sweep(&self, f: &mut Frame, center: (f32, f32)) {
        let (w, h) = f.size();
        let base = if f.apple() { GREEN } else { CYAN };
        for y in 0..h {
            for x in 0..w {
                let dx = x as f32 - center.0;
                let dy = (y as f32 - center.1) * ASPECT;
                let r = dx.hypot(dy);
                let d = behind(dy.atan2(dx), self.angle);
                if f.logo.masked(x, y) {
                    continue;
                }
                if d < TRAIL && r > 3.0 {
                    let k = (1.0 - d / TRAIL).powf(1.6);
                    let ch = if k > 0.75 {
                        '▒'
                    } else if k > 0.4 {
                        '░'
                    } else {
                        '·'
                    };
                    f.canvas.set(x, y, ch, shade(base, 0.15 + 0.75 * k), false);
                } else if r > 6.0 && ((r % 14.0) - 7.0).abs() > 6.45 && x % 2 == 0 {
                    f.canvas.set(x, y, '·', FAINT, false);
                } else if (y as f32 - center.1).abs() < 0.5 || dx.abs() < 0.5 {
                    f.canvas
                        .set(x, y, if dx.abs() < 0.5 { '│' } else { '─' }, FAINT, false);
                }
            }
        }
    }

    fn draw_blips(&mut self, f: &mut Frame, center: (f32, f32), swept: f32) {
        for i in 0..self.blips.len() {
            let b = &mut self.blips[i];
            let phi = ((b.y as f32 - center.1) * ASPECT).atan2(b.x as f32 - center.0);
            if behind(phi, self.angle) < swept + 0.001 {
                b.lit = f.t;
            }
            let age = f.t - b.lit;
            if age < 3.0 {
                let color = shade(if age < 0.15 { WHITE } else { YELLOW }, 1.0 - age / 3.0);
                f.canvas.set(b.x, b.y, '●', color, true);
            } else if age < 99.0 && self.rng.chance(f.dt * 0.4) {
                self.blips[i] = self.blip(f.logo);
            }
        }
    }
}

/// How far the beam has turned past direction `phi`, in 0..TAU.
fn behind(phi: f32, angle: f32) -> f32 {
    (angle - phi).rem_euclid(TAU)
}

impl Effect for Radar {
    fn frame(&mut self, f: &mut Frame) {
        let swept = f.dt * 1.25 * f.boost(2.2);
        self.angle += swept;
        let center = f.logo.center();
        self.draw_sweep(f, center);
        self.draw_blips(f, center, swept);
        f.logo.draw(f.canvas);
    }
}
