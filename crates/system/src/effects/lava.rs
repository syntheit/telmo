//! A lava lamp: soft metaballs drift, merge and split.

use super::palette::{BLUE, CYAN, MAGENTA, rainbow, shade};
use super::rng::Rng;
use super::{Effect, Frame};
use ratatui::style::Color;

const CHARS: [(f32, char); 6] = [
    (1.0, ' '),
    (1.25, '·'),
    (1.6, '░'),
    (2.2, '▒'),
    (3.2, '▓'),
    (1e9, '█'),
];

struct Blob {
    px: f32,
    py: f32,
    sx: f32,
    sy: f32,
    r: f32,
}

pub struct Lava {
    blobs: Vec<Blob>,
    time: f32,
}

impl Lava {
    pub fn new(mut rng: Rng) -> Lava {
        let blobs = (0..6)
            .map(|_| Blob {
                px: rng.range(0.0, 6.0),
                py: rng.range(0.0, 6.0),
                sx: rng.range(0.12, 0.25),
                sy: rng.range(0.15, 0.3),
                r: rng.range(4.5, 7.0),
            })
            .collect();
        Lava { blobs, time: 0.0 }
    }
}

impl Effect for Lava {
    fn frame(&mut self, f: &mut Frame) {
        self.time += f.dt * f.boost(2.5);
        let (w, h) = f.size();
        let (wf, hf) = (w as f32, h as f32);
        let colors: [Color; 6] = if f.apple() {
            rainbow()
        } else {
            [BLUE, CYAN, MAGENTA, BLUE, CYAN, MAGENTA]
        };
        let pos: Vec<(f32, f32, f32, Color)> = self
            .blobs
            .iter()
            .zip(colors)
            .map(|(b, c)| {
                let x = wf / 2.0 + (self.time * b.sx + b.px).sin() * (wf / 2.0 - 6.0);
                let y = hf / 2.0 + (self.time * b.sy + b.py).sin() * (hf / 2.0 - 1.0);
                (x, y, b.r, c)
            })
            .collect();
        for y in 0..h {
            for x in 0..w {
                if f.logo.masked(x, y) {
                    continue;
                }
                let (mut sum, mut best, mut color) = (0.0, 0.0, pos[0].3);
                for &(bx, by, r, c) in &pos {
                    let (dx, dy) = ((x as f32 - bx) / 2.0, y as f32 - by);
                    let v = r * r / (dx * dx * 4.0 + dy * dy * 4.0 + 1.0);
                    sum += v;
                    if v > best {
                        best = v;
                        color = c;
                    }
                }
                let ch = CHARS
                    .iter()
                    .find(|(limit, _)| sum < *limit)
                    .map_or('█', |c| c.1);
                if ch != ' ' {
                    f.canvas
                        .set(x, y, ch, shade(color, (0.35 + sum / 5.0).min(1.0)), false);
                }
            }
        }
        f.logo.draw(f.canvas);
    }
}
