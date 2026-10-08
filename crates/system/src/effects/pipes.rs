//! The pipes screensaver in rounded box lines. Each pipe is a snake that
//! fades toward its tail, so the screen never fills up.

use super::palette::{BLUE, CYAN, FG, GREEN, MAGENTA, ORANGE, shade};
use super::rng::Rng;
use super::{Effect, Frame, Logo};
use ratatui::style::Color;
use std::collections::VecDeque;

const DX: [i32; 4] = [1, 0, -1, 0];
const DY: [i32; 4] = [0, 1, 0, -1];
const TRAIL: usize = 120;
const STEP: f32 = 1.0 / 25.0;

#[derive(Clone, Copy)]
struct Seg {
    x: i32,
    y: i32,
    ch: char,
}

type Trail = VecDeque<Seg>;

struct Pipe {
    x: i32,
    y: i32,
    dir: usize,
    color: usize,
    trail: Trail,
}

pub struct Pipes {
    rng: Rng,
    pipes: Vec<Pipe>,
    /// Trails of pipes that got stuck and respawned; they keep shrinking.
    ghosts: Vec<Trail>,
    acc: f32,
}

/// The corner glyph for coming in with direction `a` and leaving with `b`.
fn corner(a: usize, b: usize) -> char {
    match (a, b) {
        (0, 1) | (3, 2) => '╮',
        (0, 3) | (1, 2) => '╯',
        (2, 1) | (3, 0) => '╭',
        _ => '╰',
    }
}

fn blocked(logo: &Logo, x: i32, y: i32) -> bool {
    let (w, h) = logo.canvas_size();
    x < 0 || x >= w || y < 0 || y >= h || logo.masked(x, y)
}

impl Pipes {
    pub fn new(logo: &Logo, mut rng: Rng) -> Pipes {
        let pipes = (0..4).map(|i| spawn(logo, &mut rng, i)).collect();
        Pipes {
            rng,
            pipes,
            ghosts: Vec::new(),
            acc: 0.0,
        }
    }

    fn turn(&mut self, dir: usize) -> usize {
        (dir + if self.rng.chance(0.5) { 1 } else { 3 }) % 4
    }

    /// The next direction: usually straight on, sometimes a turn, and a turn
    /// or a U-turn when something is in the way. None when boxed in.
    fn choose(&mut self, logo: &Logo, p: &Pipe) -> Option<usize> {
        let ahead = |d: usize| blocked(logo, p.x + DX[d], p.y + DY[d]);
        let mut d = p.dir;
        if self.rng.chance(0.12) {
            d = self.turn(d);
        }
        if ahead(d) {
            d = self.turn(p.dir);
        }
        if ahead(d) {
            d = (d + 2) % 4;
        }
        (!ahead(d)).then_some(d)
    }

    fn step(&mut self, logo: &Logo) {
        for n in 0..self.pipes.len() {
            let mut p = std::mem::replace(
                &mut self.pipes[n],
                Pipe {
                    x: 0,
                    y: 0,
                    dir: 0,
                    color: 0,
                    trail: Trail::new(),
                },
            );
            let Some(d) = self.choose(logo, &p) else {
                self.ghosts.push(p.trail);
                self.pipes[n] = spawn(logo, &mut self.rng, p.color);
                continue;
            };
            let ch = if d % 2 == p.dir % 2 {
                if d % 2 == 1 { '│' } else { '─' }
            } else {
                corner(p.dir, d)
            };
            p.trail.push_back(Seg { x: p.x, y: p.y, ch });
            if p.trail.len() > TRAIL {
                p.trail.pop_front();
            }
            p.x += DX[d];
            p.y += DY[d];
            p.dir = d;
            self.pipes[n] = p;
        }
        for g in &mut self.ghosts {
            g.pop_front();
        }
        self.ghosts.retain(|g| !g.is_empty());
    }

    #[cfg(test)]
    fn trail_cells(&self) -> usize {
        self.pipes
            .iter()
            .map(|p| p.trail.len())
            .chain(self.ghosts.iter().map(|g| g.len()))
            .sum()
    }
}

fn spawn(logo: &Logo, rng: &mut Rng, color: usize) -> Pipe {
    let (w, h) = logo.canvas_size();
    let mut pipe = Pipe {
        x: 0,
        y: 0,
        dir: rng.below(4),
        color,
        trail: Trail::new(),
    };
    for _ in 0..200 {
        pipe.x = rng.below(w as usize) as i32;
        pipe.y = rng.below(h as usize) as i32;
        if !logo.masked(pipe.x, pipe.y) {
            break;
        }
    }
    pipe
}

fn paint(f: &mut Frame, trail: &Trail, color: Color) {
    for (i, s) in trail.iter().enumerate() {
        let k = (i + 1) as f32 / TRAIL as f32;
        f.canvas.set(
            s.x,
            s.y,
            s.ch,
            shade(color, 0.12 + 0.78 * k.powf(0.8)),
            false,
        );
    }
}

impl Effect for Pipes {
    fn frame(&mut self, f: &mut Frame) {
        let colors = if f.apple() {
            [GREEN, ORANGE, MAGENTA, BLUE]
        } else {
            [BLUE, CYAN, MAGENTA, FG]
        };
        self.acc += f.dt * f.boost(2.5);
        while self.acc > STEP {
            self.acc -= STEP;
            self.step(f.logo);
        }
        for (i, g) in self.ghosts.iter().enumerate() {
            paint(f, g, colors[i % colors.len()]);
        }
        for p in &self.pipes {
            paint(f, &p.trail, colors[p.color]);
            f.canvas.set(p.x, p.y, '●', colors[p.color], true);
        }
        f.logo.draw(f.canvas);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::canvas::Canvas;
    use crate::effects::LogoKind;

    #[test]
    fn trails_stay_bounded() {
        for kind in [LogoKind::Apple, LogoKind::Nix] {
            let mut logo = Logo::place(kind, 90, 21);
            let mut canvas = Canvas::new(90, 21);
            let mut pipes = Pipes::new(&logo, Rng::new(9));
            let dt = 1.0 / 60.0;
            let mut worst = 0;
            for i in 0..3600 {
                canvas.clear();
                let mut f = Frame {
                    canvas: &mut canvas,
                    logo: &mut logo,
                    t: i as f32 * dt,
                    dt,
                    busy: i % 600 > 300,
                    finished: false,
                };
                pipes.frame(&mut f);
                worst = worst.max(pipes.trail_cells());
            }
            assert!(worst <= 4 * TRAIL + 4 * TRAIL, "{worst} trail cells");
            assert!(worst > 100);
        }
    }
}
