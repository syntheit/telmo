//! Digital rain: a drop per column leaves a fading trail of glyphs.

use super::palette::{BLUE, CYAN, WHITE, glyph, rainbow, shade};
use super::rng::Rng;
use super::{Effect, Frame, Logo};

struct Drop {
    y: f32,
    v: f32,
    last: i32,
}

pub struct Rain {
    rng: Rng,
    drops: Vec<Drop>,
    intensity: Vec<f32>,
    glyphs: Vec<char>,
}

impl Rain {
    pub fn new(logo: &Logo, mut rng: Rng) -> Rain {
        let (w, h) = logo.canvas_size();
        let popup = (h + 1) as f32;
        let drops = (0..w)
            .map(|_| Drop {
                y: rng.range(-2.0 * popup, popup),
                v: rng.range(6.0, 16.0),
                last: -1,
            })
            .collect();
        Rain {
            rng,
            drops,
            intensity: vec![0.0; (w * h) as usize],
            glyphs: vec![' '; (w * h) as usize],
        }
    }

    fn fade(&mut self, dt: f32, speed: f32) {
        let decay = (-dt * 2.6 * speed).exp();
        let churn = 0.6 * dt;
        for (v, g) in self.intensity.iter_mut().zip(&mut self.glyphs) {
            *v *= decay;
            if self.rng.chance(churn) {
                *g = glyph(&mut self.rng);
            }
        }
    }

    fn fall(&mut self, dt: f32, speed: f32, w: i32, h: i32) {
        for (x, d) in self.drops.iter_mut().enumerate() {
            d.y += d.v * dt * speed;
            let row = d.y.floor() as i32;
            if row != d.last && row >= 0 && row < h {
                let i = (row * w) as usize + x;
                self.intensity[i] = 1.0;
                self.glyphs[i] = glyph(&mut self.rng);
                d.last = row;
            }
            if d.y > (h + 1 + 8) as f32 {
                d.y = self.rng.range(-30.0, -1.0);
                d.v = self.rng.range(6.0, 16.0);
                d.last = -1;
            }
        }
    }
}

impl Effect for Rain {
    fn frame(&mut self, f: &mut Frame) {
        let (w, h) = f.size();
        let speed = f.boost(2.4);
        self.fade(f.dt, speed);
        self.fall(f.dt, speed, w, h);
        let bands = rainbow();
        for y in 0..h {
            for x in 0..w {
                let i = (y * w + x) as usize;
                let v = self.intensity[i];
                if v < 0.06 {
                    continue;
                }
                let base = if f.apple() {
                    bands[(((y as f32 / h as f32) * 6.0).floor() as usize).min(5)]
                } else if x % 3 == 0 {
                    CYAN
                } else {
                    BLUE
                };
                let hot = v > 0.97;
                let color = if hot {
                    WHITE
                } else {
                    shade(base, (v * 0.9).min(1.0))
                };
                f.put(x, y, self.glyphs[i], color, hot);
            }
        }
        f.logo.draw(f.canvas);
    }
}
