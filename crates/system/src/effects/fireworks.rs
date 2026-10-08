//! Rockets climb from the bottom and burst into falling sparks.

use super::palette::{BLUE, CYAN, ICE, MAGENTA, WHITE, YELLOW, rainbow, round, shade};
use super::rng::Rng;
use super::{Effect, Frame};
use ratatui::style::Color;
use std::f32::consts::TAU;

struct Rocket {
    x: f32,
    y: f32,
    vx: f32,
    vy: f32,
    color: Color,
}

struct Spark {
    x: f32,
    y: f32,
    vx: f32,
    vy: f32,
    life: f32,
    age: f32,
    color: Color,
}

pub struct Fireworks {
    rng: Rng,
    rockets: Vec<Rocket>,
    sparks: Vec<Spark>,
    next: f32,
}

impl Fireworks {
    pub fn new(rng: Rng) -> Fireworks {
        Fireworks {
            rng,
            rockets: Vec::new(),
            sparks: Vec::new(),
            next: 0.0,
        }
    }

    fn launch(&mut self, f: &Frame, palette: &[Color]) {
        let (w, h) = f.size();
        let rocket = Rocket {
            x: self.rng.range(6.0, (w - 6) as f32),
            y: (h - 1) as f32,
            vx: self.rng.range(-4.0, 4.0),
            vy: -self.rng.range(16.0, 22.0),
            color: self.rng.pick(palette),
        };
        self.rockets.push(rocket);
    }

    fn burst(&mut self, r: &Rocket) {
        let n = self.rng.range(40.0, 60.0).floor() as usize;
        let speed = self.rng.range(10.0, 16.0);
        for k in 0..n {
            let a = (k as f32 / n as f32) * TAU + self.rng.range(-0.1, 0.1);
            let v = speed * self.rng.range(0.6, 1.0);
            let color = if self.rng.chance(0.2) { WHITE } else { r.color };
            let life = self.rng.range(1.1, 1.8);
            self.sparks.push(Spark {
                x: r.x,
                y: r.y,
                vx: a.cos() * v,
                vy: a.sin() * v * 0.5,
                life,
                age: 0.0,
                color,
            });
        }
    }

    fn update(&mut self, dt: f32) {
        for r in &mut self.rockets {
            r.x += r.vx * dt;
            r.y += r.vy * dt;
            r.vy += 14.0 * dt;
        }
        let (rising, bursting): (Vec<Rocket>, Vec<Rocket>) =
            self.rockets.drain(..).partition(|r| r.vy <= -2.0);
        self.rockets = rising;
        for r in &bursting {
            self.burst(r);
        }
        let drag = (-dt * 1.4).exp();
        for p in &mut self.sparks {
            p.age += dt;
            p.x += p.vx * dt;
            p.y += p.vy * dt;
            p.vx *= drag;
            p.vy = p.vy * drag + 5.0 * dt;
        }
        self.sparks.retain(|p| p.age < p.life);
    }
}

impl Effect for Fireworks {
    fn frame(&mut self, f: &mut Frame) {
        let palette: Vec<Color> = if f.apple() {
            rainbow().to_vec()
        } else {
            vec![BLUE, CYAN, MAGENTA, ICE]
        };
        if f.t > self.next {
            self.launch(f, &palette);
            self.next = f.t
                + if f.busy {
                    self.rng.range(0.15, 0.5)
                } else {
                    self.rng.range(0.5, 1.4)
                };
        }
        if f.finished {
            for _ in 0..9 {
                self.launch(f, &palette);
            }
        }
        self.update(f.dt);
        for r in &self.rockets {
            let (x, y) = (round(r.x), round(r.y));
            f.put(x, y + 1, '·', shade(YELLOW, 0.4), false);
            f.put(x, y, '^', YELLOW, true);
        }
        for p in &self.sparks {
            let k = 1.0 - p.age / p.life;
            let ch = if k > 0.6 {
                '*'
            } else if k > 0.3 {
                '+'
            } else {
                '·'
            };
            f.put(
                round(p.x),
                round(p.y),
                ch,
                shade(p.color, 0.25 + 0.75 * k),
                k > 0.6,
            );
        }
        f.logo.draw(f.canvas);
    }
}
