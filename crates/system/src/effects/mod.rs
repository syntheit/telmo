//! The animated backgrounds. Each effect draws one frame at a time into a
//! `Canvas`, around (and usually behind) the logo. They are sized for the
//! logo's canvas: after a resize, place the logo again and call `make` again.

mod aurora;
mod dvd;
mod fire;
mod fireflies;
mod fireworks;
mod galaxy;
mod glitch;
mod lava;
pub mod logo;
mod palette;
mod pipes;
mod plasma;
mod radar;
mod rain;
mod rng;
mod snow;
mod tunnel;
mod warp;

use crate::canvas::Canvas;
pub use logo::{Logo, LogoKind};
use palette::{BLUE, shade};
use ratatui::style::Color;
use rng::Rng;

/// Everything an effect needs for one frame.
pub struct Frame<'a> {
    pub canvas: &'a mut Canvas,
    pub logo: &'a mut Logo,
    /// Seconds since the effect started.
    pub t: f32,
    /// Seconds since the previous frame (clamped to at most 0.1).
    pub dt: f32,
    /// A rebuild is running: effects speed up.
    pub busy: bool,
    /// True on the single frame when a rebuild finished successfully.
    pub finished: bool,
}

impl Frame<'_> {
    pub(crate) fn size(&self) -> (i32, i32) {
        (self.canvas.width as i32, self.canvas.height as i32)
    }

    pub(crate) fn apple(&self) -> bool {
        self.logo.kind == LogoKind::Apple
    }

    /// `speed` while a rebuild runs, otherwise 1.
    pub(crate) fn boost(&self, speed: f32) -> f32 {
        if self.busy { speed } else { 1.0 }
    }

    /// Draws behind the logo: skips the logo's mask.
    pub(crate) fn put(&mut self, x: i32, y: i32, ch: char, color: Color, bold: bool) {
        if !self.logo.masked(x, y) {
            self.canvas.set(x, y, ch, color, bold);
        }
    }
}

pub trait Effect {
    /// Draws the background and the logo for one frame. The canvas starts cleared.
    fn frame(&mut self, f: &mut Frame);
}

/// Every effect's name, in cycle order. Names are what the state file stores.
pub const NAMES: [&str; 15] = [
    "rain",
    "plasma",
    "glitch",
    "warp",
    "fire",
    "radar",
    "snow",
    "fireworks",
    "pipes",
    "lava",
    "dvd",
    "tunnel",
    "aurora",
    "fireflies",
    "galaxy",
];

/// A fresh effect by name; unknown names fall back to the first one.
pub fn make(name: &str, logo: &Logo) -> Box<dyn Effect> {
    make_seeded(name, logo, rng::clock_seed())
}

pub fn make_seeded(name: &str, logo: &Logo, seed: u64) -> Box<dyn Effect> {
    let rng = Rng::new(seed);
    match name {
        "plasma" => Box::new(plasma::Plasma::new()),
        "glitch" => Box::new(glitch::Glitch::new(rng)),
        "warp" => Box::new(warp::Warp::new(rng)),
        "fire" => Box::new(fire::Fire::new(logo, rng)),
        "radar" => Box::new(radar::Radar::new(logo, rng)),
        "snow" => Box::new(snow::Snow::new(logo, rng)),
        "fireworks" => Box::new(fireworks::Fireworks::new(rng)),
        "pipes" => Box::new(pipes::Pipes::new(logo, rng)),
        "lava" => Box::new(lava::Lava::new(rng)),
        "dvd" => Box::new(dvd::Dvd::new(rng)),
        "tunnel" => Box::new(tunnel::Tunnel::new()),
        "aurora" => Box::new(aurora::Aurora::new(logo, rng)),
        "fireflies" => Box::new(fireflies::Fireflies::new(logo, rng)),
        "galaxy" => Box::new(galaxy::Galaxy::new(rng)),
        _ => Box::new(rain::Rain::new(logo, rng)),
    }
}

/// The decrypt-in that plays when the popup opens and on every switch: logo
/// cells show random glyphs until their reveal time.
pub struct Transition {
    reveal: Vec<f32>,
}

impl Transition {
    pub fn new(logo: &Logo) -> Transition {
        let mut rng = Rng::from_clock();
        let reveal = logo
            .cells
            .iter()
            .map(|c| (c.lx as f32 / logo.width.max(1) as f32) * 0.35 + rng.range(0.0, 0.2))
            .collect();
        Transition { reveal }
    }

    /// `t` is seconds since the switch. Drawn after the effect's frame.
    pub fn apply(&self, canvas: &mut Canvas, logo: &Logo, t: f32) {
        let frame = (t * 60.0) as u64;
        for (i, (cell, reveal)) in logo.cells.iter().zip(&self.reveal).enumerate() {
            if t >= *reveal {
                continue;
            }
            // Different glyph every frame, without needing mutable state.
            let mut rng = Rng::new(frame << 20 ^ i as u64);
            let color = if t > reveal - 0.15 {
                cell.color
            } else {
                shade(BLUE, 0.5)
            };
            canvas.set(
                cell.x + logo.offset.0,
                cell.y + logo.offset.1,
                palette::glyph(&mut rng),
                color,
                false,
            );
        }
    }

    pub fn done(&self, t: f32) -> bool {
        self.reveal.iter().all(|r| t >= *r)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Instant;

    const KINDS: [LogoKind; 2] = [LogoKind::Apple, LogoKind::Nix];

    fn run(name: &str, kind: LogoKind, dt: f32) -> (Canvas, Logo) {
        let mut logo = Logo::place(kind, 90, 21);
        let mut canvas = Canvas::new(90, 21);
        let mut effect = make_seeded(name, &logo, 7);
        for i in 0..400 {
            canvas.clear();
            let mut f = Frame {
                canvas: &mut canvas,
                logo: &mut logo,
                t: i as f32 * dt,
                dt,
                busy: i >= 300,
                finished: i == 350,
            };
            effect.frame(&mut f);
        }
        (canvas, logo)
    }

    #[test]
    fn every_effect_runs_and_draws_the_logo() {
        for name in NAMES {
            for kind in KINDS {
                for dt in [1.0 / 60.0, 1.0 / 30.0] {
                    let (canvas, logo) = run(name, kind, dt);
                    let drawn = |c: &logo::LogoCell| {
                        canvas
                            .get(c.x + logo.offset.0, c.y + logo.offset.1)
                            .is_some_and(|d| d.ch == c.ch)
                    };
                    // DVD bounces it and glitch shifts rows, so only check those loosely.
                    let loose = name == "dvd" || name == "glitch";
                    let present = if loose {
                        logo.cells
                            .iter()
                            .any(|c| canvas.get(c.x + logo.offset.0, c.y).is_some())
                    } else {
                        logo.cells.iter().all(drawn)
                    };
                    assert!(present, "{name} {kind:?} dt={dt}");
                }
            }
        }
    }

    #[test]
    fn unknown_name_falls_back() {
        let mut logo = Logo::place(LogoKind::Apple, 90, 21);
        let mut canvas = Canvas::new(90, 21);
        let mut effect = make("nonsense", &logo);
        let mut f = Frame {
            canvas: &mut canvas,
            logo: &mut logo,
            t: 0.0,
            dt: 0.016,
            busy: false,
            finished: false,
        };
        effect.frame(&mut f);
    }

    #[test]
    fn transition_finishes_within_a_second() {
        let logo = Logo::place(LogoKind::Nix, 90, 21);
        let transition = Transition::new(&logo);
        assert!(!transition.done(0.0));
        assert!(transition.done(0.6));
        let mut canvas = Canvas::new(90, 21);
        transition.apply(&mut canvas, &logo, 0.0);
        assert!(logo.cells.iter().all(|c| canvas.get(c.x, c.y).is_some()));
    }

    #[test]
    #[ignore]
    fn frame_cost() {
        for name in NAMES {
            let mut logo = Logo::place(LogoKind::Apple, 90, 21);
            let mut canvas = Canvas::new(90, 21);
            let mut effect = make_seeded(name, &logo, 1);
            let start = Instant::now();
            for i in 0..1000 {
                canvas.clear();
                let mut f = Frame {
                    canvas: &mut canvas,
                    logo: &mut logo,
                    t: i as f32 / 30.0,
                    dt: 1.0 / 30.0,
                    busy: false,
                    finished: false,
                };
                effect.frame(&mut f);
            }
            println!(
                "{name:10} {:7.1} us/frame",
                start.elapsed().as_secs_f64() * 1000.0
            );
        }
    }
}
