//! The demoscene fire: a heat grid that rises and cools.

use super::palette::{BLUE, CYAN, HOT_WHITE, ICE, MAGENTA, ORANGE, RED, YELLOW, shade};
use super::rng::Rng;
use super::{Effect, Frame, Logo};
use ratatui::style::Color;

const STEP: f32 = 1.0 / 30.0;
const CHARS: [(f32, char); 6] = [
    (0.1, '.'),
    (0.2, ':'),
    (0.32, '░'),
    (0.48, '▒'),
    (0.64, '▓'),
    (2.0, '█'),
];

pub struct Fire {
    rng: Rng,
    heat: Vec<f32>,
    acc: f32,
}

impl Fire {
    pub fn new(logo: &Logo, rng: Rng) -> Fire {
        let (w, h) = logo.canvas_size();
        Fire {
            rng,
            heat: vec![0.0; (w * h) as usize],
            acc: 0.0,
        }
    }

    fn step(&mut self, w: i32, h: i32, hot: f32, cool: f32) {
        for x in 0..w {
            let v = if self.rng.chance(0.55) {
                self.rng.range(0.75, 1.0) * hot
            } else {
                self.rng.range(0.0, 0.35)
            };
            self.heat[((h - 1) * w + x) as usize] = v;
        }
        for y in 0..h - 1 {
            for x in 0..w {
                let at =
                    |yy: i32, xx: i32| self.heat[(yy.min(h - 1) * w + xx.rem_euclid(w)) as usize];
                let v = (at(y + 1, x - 1) + at(y + 1, x) + at(y + 1, x + 1) + at(y + 2, x)) / 4.0;
                self.heat[(y * w + x) as usize] = (v - self.rng.range(0.0, cool)).max(0.0);
            }
        }
    }
}

fn ramp(apple: bool) -> [Color; 5] {
    if apple {
        [shade(RED, 0.35), RED, ORANGE, YELLOW, HOT_WHITE]
    } else {
        [shade(MAGENTA, 0.35), MAGENTA, BLUE, CYAN, ICE]
    }
}

impl Effect for Fire {
    fn frame(&mut self, f: &mut Frame) {
        let (w, h) = f.size();
        let (hot, cool) = if f.busy { (1.0, 0.075) } else { (0.8, 0.11) };
        self.acc += f.dt;
        while self.acc > STEP {
            self.acc -= STEP;
            self.step(w, h, hot, cool);
        }
        let ramp = ramp(f.apple());
        for y in 0..h {
            for x in 0..w {
                let v = self.heat[(y * w + x) as usize];
                if v < 0.04 {
                    continue;
                }
                let ch = CHARS
                    .iter()
                    .find(|(limit, _)| v < *limit)
                    .map_or('█', |c| c.1);
                let k = ((v * ramp.len() as f32) as usize).min(ramp.len() - 1);
                f.put(x, y, ch, ramp[k], false);
            }
        }
        f.logo.draw(f.canvas);
    }
}
