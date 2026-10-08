//! A still logo with a scanline, and short bursts where rows jump sideways.

use super::palette::{CYAN, FAINT, RED, WHITE, round};
use super::rng::Rng;
use super::{Effect, Frame};
use ratatui::style::Color;

struct Jump {
    y: i32,
    dx: i32,
    color: Color,
}

pub struct Glitch {
    rng: Rng,
    next: f32,
    until: f32,
    jumps: Vec<Jump>,
}

impl Glitch {
    pub fn new(rng: Rng) -> Glitch {
        Glitch {
            rng,
            next: 0.0,
            until: 0.0,
            jumps: Vec::new(),
        }
    }

    fn start_burst(&mut self, f: &Frame) {
        let every = if f.busy { 1.1 } else { 2.8 };
        self.until = f.t + 0.22;
        self.next = f.t + every * self.rng.range(0.7, 1.3);
        self.jumps = (0..5)
            .map(|_| Jump {
                y: f.logo.oy + self.rng.below(f.logo.height as usize) as i32,
                dx: round(self.rng.range(-5.0, 5.0)),
                color: if self.rng.chance(0.5) { RED } else { CYAN },
            })
            .collect();
    }

    fn noise(&mut self, f: &mut Frame) {
        let (w, h) = f.size();
        // About 360 specks a second, whatever the frame rate.
        for _ in 0..self.rng.count(360.0 * f.dt) {
            let (x, y) = (
                self.rng.below(w as usize) as i32,
                self.rng.below(h as usize) as i32,
            );
            f.put(x, y, '░', FAINT, false);
        }
    }
}

impl Effect for Glitch {
    fn frame(&mut self, f: &mut Frame) {
        if f.t > self.next {
            self.start_burst(f);
        }
        let glitching = f.t < self.until;
        let scan = f.logo.oy as f32 + ((f.t * 5.0) % (f.logo.height + 10) as f32) - 5.0;
        for c in &f.logo.cells {
            // A later jump on the same row wins.
            let jump = if glitching {
                self.jumps.iter().rev().find(|j| j.y == c.y)
            } else {
                None
            };
            match jump {
                Some(j) => {
                    let ch = if self.rng.chance(0.25) {
                        self.rng.pick(&['▓', '▒', '░', '█'])
                    } else {
                        c.ch
                    };
                    f.canvas.set(c.x + j.dx, c.y, ch, j.color, true);
                }
                None => {
                    let near = (c.y as f32 - scan).abs() < 1.0;
                    f.canvas
                        .set(c.x, c.y, c.ch, if near { WHITE } else { c.color }, true);
                }
            }
        }
        self.noise(f);
    }
}
