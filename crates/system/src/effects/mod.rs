//! The animated backgrounds. Each effect draws one frame at a time into a
//! `Canvas`, around (and usually behind) the logo.
//!
//! STUB: the real effects replace this file's `Blank` (see docs/mockups/system-effects.template.html).

pub mod logo;

use crate::canvas::Canvas;
pub use logo::{Logo, LogoKind};

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

pub trait Effect {
    /// Draws the background and the logo for one frame. The canvas starts cleared.
    fn frame(&mut self, f: &mut Frame);
}

/// Every effect's name, in cycle order. Names are what the state file stores.
pub const NAMES: [&str; 15] = [
    "rain", "plasma", "glitch", "warp", "fire", "radar", "snow", "fireworks", "pipes", "lava", "dvd",
    "tunnel", "aurora", "fireflies", "galaxy",
];

/// A fresh effect by name; unknown names fall back to the first one.
pub fn make(name: &str, logo: &Logo) -> Box<dyn Effect> {
    let _ = (name, logo);
    Box::new(Blank)
}

struct Blank;

impl Effect for Blank {
    fn frame(&mut self, f: &mut Frame) {
        f.logo.draw(f.canvas);
    }
}

/// The decrypt-in that plays when the popup opens and on every switch: logo
/// cells show random glyphs until their reveal time.
pub struct Transition {
    reveal: Vec<f32>,
}

impl Transition {
    pub fn new(logo: &Logo) -> Transition {
        Transition { reveal: vec![0.0; logo.cells.len()] }
    }

    /// `t` is seconds since the switch. Drawn after the effect's frame.
    pub fn apply(&self, canvas: &mut Canvas, logo: &Logo, t: f32) {
        let _ = (canvas, logo, t, &self.reveal);
    }

    pub fn done(&self, t: f32) -> bool {
        self.reveal.iter().all(|r| t >= *r)
    }
}
