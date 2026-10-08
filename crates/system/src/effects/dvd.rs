//! The logo bounces around like the DVD screensaver; a corner hit celebrates.

use super::palette::{BLUE, CYAN, FG, GREEN, MAGENTA, ORANGE, RED, YELLOW, rainbow, round, shade};
use super::rng::Rng;
use super::{Effect, Frame};
use ratatui::style::Color;
use std::f32::consts::TAU;

struct Spark {
    x: f32,
    y: f32,
    vx: f32,
    vy: f32,
    age: f32,
    color: Color,
}

pub struct Dvd {
    rng: Rng,
    x: f32,
    y: f32,
    vx: f32,
    vy: f32,
    hue: usize,
    party: f32,
    sparks: Vec<Spark>,
}

impl Dvd {
    pub fn new(rng: Rng) -> Dvd {
        Dvd {
            rng,
            x: 0.0,
            y: 0.0,
            vx: 9.0,
            vy: 4.2,
            hue: 0,
            party: -9.0,
            sparks: Vec::new(),
        }
    }

    /// Moves and bounces; returns how many walls were hit.
    fn travel(&mut self, f: &Frame, speed: f32) -> u32 {
        let (w, h) = f.size();
        let l = &f.logo;
        let (min_x, max_x) = (-l.ox as f32, (w - l.width - l.ox) as f32);
        let (min_y, max_y) = (-l.oy as f32, (h - l.height - l.oy) as f32);
        self.x += self.vx * f.dt * speed;
        self.y += self.vy * f.dt * speed;
        let mut hits = 0;
        if self.x < min_x || self.x > max_x {
            self.vx = -self.vx;
            self.x = self.x.min(max_x).max(min_x);
            hits += 1;
        }
        if self.y < min_y || self.y > max_y {
            self.vy = -self.vy;
            self.y = self.y.min(max_y).max(min_y);
            hits += 1;
        }
        hits
    }

    fn celebrate(&mut self, f: &Frame) {
        self.party = f.t;
        let (cx, cy) = f.logo.center();
        let colors = [RED, ORANGE, GREEN, YELLOW, BLUE, MAGENTA, CYAN, FG];
        for _ in 0..70 {
            let (a, v) = (self.rng.range(0.0, TAU), self.rng.range(8.0, 20.0));
            let color = self.rng.pick(&colors);
            let spark = Spark {
                x: cx + self.x,
                y: cy + self.y,
                vx: a.cos() * v,
                vy: a.sin() * v * 0.5,
                age: 0.0,
                color,
            };
            self.sparks.push(spark);
        }
    }

    fn draw_sparks(&mut self, f: &mut Frame) {
        let h = f.size().1;
        for p in &mut self.sparks {
            p.age += f.dt;
            p.x += p.vx * f.dt;
            p.y += p.vy * f.dt;
            p.vy += 6.0 * f.dt;
        }
        self.sparks.retain(|p| p.age < 1.6);
        for p in &self.sparks {
            let y = round(p.y);
            if y >= 0 && y < h {
                let ch = if p.age < 0.6 { '*' } else { '·' };
                f.canvas
                    .set(round(p.x), y, ch, shade(p.color, 1.0 - p.age / 1.6), false);
            }
        }
    }
}

impl Effect for Dvd {
    fn frame(&mut self, f: &mut Frame) {
        let hits = self.travel(f, f.boost(2.0));
        if hits > 0 {
            self.hue += 1;
        }
        if hits == 2 {
            self.celebrate(f);
        }
        f.logo.offset = (round(self.x), round(self.y));
        self.draw_sparks(f);
        let colors: Vec<Color> = if f.apple() {
            rainbow().to_vec()
        } else {
            vec![BLUE, CYAN, MAGENTA, GREEN]
        };
        let party = f.t - self.party < 1.2;
        let offset = f.logo.offset;
        for c in &f.logo.cells {
            let color = if party {
                self.rng.pick(&rainbow())
            } else {
                colors[self.hue % colors.len()]
            };
            f.canvas
                .set(c.x + offset.0, c.y + offset.1, c.ch, color, true);
        }
        if party {
            let msg = "corner!";
            let (w, h) = f.size();
            f.canvas
                .text((w - msg.len() as i32) / 2, h - 1, msg, YELLOW, true);
        }
    }
}
