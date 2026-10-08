//! Snow drifts down and settles on the floor, the logo and itself, then melts.

use super::palette::{SNOW_BLUE, WHITE, shade};
use super::rng::Rng;
use super::{Effect, Frame, Logo};

struct Flake {
    x: f32,
    y: f32,
    v: f32,
    drift: f32,
    phase: f32,
    big: bool,
}

pub struct Snow {
    rng: Rng,
    flakes: Vec<Flake>,
    settled: Vec<bool>,
    count: i32,
    melting: bool,
    /// Fractional flakes owed: at 60 fps a single frame asks for less than one.
    due: f32,
    width: i32,
    height: i32,
}

impl Snow {
    pub fn new(logo: &Logo, rng: Rng) -> Snow {
        let (width, height) = logo.canvas_size();
        Snow {
            rng,
            flakes: Vec::new(),
            settled: vec![false; (width * height) as usize],
            count: 0,
            melting: false,
            due: 0.0,
            width,
            height,
        }
    }

    fn is_settled(&self, x: i32, y: i32) -> bool {
        x >= 0
            && y >= 0
            && x < self.width
            && y < self.height
            && self.settled[(y * self.width + x) as usize]
    }

    fn blocked(&self, logo: &Logo, x: i32, y: i32) -> bool {
        y >= self.height || (y >= 0 && (logo.masked(x, y) || self.is_settled(x, y)))
    }

    fn spawn(&mut self, dt: f32, rate: f32) {
        self.due += rate * dt;
        while self.due >= 1.0 {
            self.due -= 1.0;
            let flake = Flake {
                x: self.rng.range(0.0, self.width as f32),
                y: -1.0,
                v: self.rng.range(2.5, 5.0),
                drift: self.rng.range(0.6, 1.6),
                phase: self.rng.range(0.0, 6.0),
                big: self.rng.chance(0.25),
            };
            self.flakes.push(flake);
        }
    }

    /// Moves one flake; returns false once it has settled.
    fn fall(&mut self, flake: &mut Flake, logo: &Logo, t: f32, dt: f32) -> bool {
        let w = self.width as f32;
        let ny = flake.y + flake.v * dt;
        let nx = (flake.x + (t * flake.drift + flake.phase).sin() * dt * 2.0 + w).rem_euclid(w);
        let (cx, cy) = (nx.floor() as i32, ny.floor() as i32);
        if cy < 0 || !self.blocked(logo, cx, cy) {
            flake.x = nx;
            flake.y = ny;
            return true;
        }
        // Slide down a free side first, like sand, so snow forms mounds instead of towers.
        let (first, second) = if self.rng.chance(0.5) {
            (cx - 1, cx + 1)
        } else {
            (cx + 1, cx - 1)
        };
        let free = |s: i32| {
            s >= 0 && s < self.width && !self.blocked(logo, s, cy) && !self.blocked(logo, s, cy - 1)
        };
        if let Some(side) = [first, second].into_iter().find(|s| free(*s)) {
            flake.x = side as f32 + 0.5;
            flake.y = cy as f32 - 0.5;
            return true;
        }
        self.land(logo, cx, cy - 1);
        false
    }

    /// Piles are at most two deep.
    fn land(&mut self, logo: &Logo, x: i32, y: i32) {
        let deep = self.is_settled(x, y + 1) && self.is_settled(x, (y + 2).min(self.height - 1));
        if y >= 0 && !self.blocked(logo, x, y) && !deep {
            self.settled[(y * self.width + x) as usize] = true;
            self.count += 1;
        }
    }

    fn update(&mut self, f: &Frame) {
        self.spawn(f.dt, if f.busy { 70.0 } else { 28.0 });
        let mut flakes = std::mem::take(&mut self.flakes);
        flakes.retain_mut(|flake| {
            self.fall(flake, f.logo, f.t, f.dt) && flake.y < self.height as f32
        });
        self.flakes = flakes;
        if self.count > 300 {
            self.melting = true;
        }
        if self.melting {
            self.melt(f.dt);
        }
    }

    fn melt(&mut self, dt: f32) {
        for i in 0..self.settled.len() {
            if self.settled[i] && self.rng.chance(dt * 0.8) {
                self.settled[i] = false;
                self.count -= 1;
            }
        }
        if self.count < 20 {
            self.melting = false;
        }
    }

    fn draw_settled(&self, f: &mut Frame, color: ratatui::style::Color) {
        let shaded = shade(color, if self.melting { 0.5 } else { 0.85 });
        for y in 0..self.height {
            for x in 0..self.width {
                if !self.is_settled(x, y) {
                    continue;
                }
                let exposed = self.blocked(f.logo, x, y + 1) && !self.is_settled(x, y + 1);
                let ch = if self.melting || exposed {
                    '▄'
                } else {
                    '█'
                };
                f.canvas.set(x, y, ch, shaded, false);
            }
        }
    }
}

impl Effect for Snow {
    fn frame(&mut self, f: &mut Frame) {
        self.update(f);
        let color = if f.apple() { WHITE } else { SNOW_BLUE };
        self.draw_settled(f, color);
        for flake in &self.flakes {
            let (x, y) = (flake.x.floor() as i32, flake.y.floor() as i32);
            if y >= 0 {
                let (ch, strength) = if flake.big { ('*', 0.95) } else { ('·', 0.6) };
                f.put(x, y, ch, shade(color, strength), false);
            }
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
    fn snow_settles_at_60_fps() {
        let mut logo = Logo::place(LogoKind::Apple, 90, 21);
        let mut canvas = Canvas::new(90, 21);
        let mut snow = Snow::new(&logo, Rng::new(5));
        let dt = 1.0 / 60.0;
        for i in 0..1200 {
            canvas.clear();
            let mut f = Frame {
                canvas: &mut canvas,
                logo: &mut logo,
                t: i as f32 * dt,
                dt,
                busy: false,
                finished: false,
            };
            snow.frame(&mut f);
            if i == 600 {
                assert!(snow.count > 50, "only {} settled after 10 s", snow.count);
            }
        }
        assert!(snow.count > 0);
    }
}
