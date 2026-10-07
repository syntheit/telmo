//! State and key handling. No drawing here.
//!
//! There is deliberately no mouse handling: touching the trackpad would click.

use crate::backend::Event;
use crate::model::Snapshot;
use crossterm::event::{KeyCode, KeyEvent};
use std::time::{Duration, Instant};
use telmo_kit::{Flow, widgets::Toast};

const GRAMS_PER_OUNCE: f32 = 28.349_523;
/// The finger alone must stay this still...
const ZERO_SPREAD: f32 = 0.8;
/// ...for this long before the scale zeroes itself.
const ZERO_AFTER: Duration = Duration::from_millis(600);
/// Below this many grams past the tare, nothing is on the scale.
const WEIGHT_MIN: f32 = 1.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Unit {
    Grams,
    Ounces,
}

/// Where the user is in the weighing routine.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Step {
    /// No touch: rest one finger on the trackpad.
    Rest,
    /// Touching, waiting for the reading to settle so it can be zeroed.
    Zeroing,
    /// Zeroed: place the item next to the finger.
    Place,
    /// Zeroed, weight on, reading steady.
    Done,
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Phase {
    Idle,
    /// Touching, not zeroed yet. The reading has been within `ZERO_SPREAD`
    /// of `anchor` since `since`.
    Settling {
        anchor: f32,
        since: Instant,
    },
    Zeroed,
}

pub struct App {
    pub snapshot: Snapshot,
    /// False until the backend has told us anything.
    pub loaded: bool,
    pub unit: Unit,
    pub help: bool,
    pub toast: Option<Toast>,
    /// Grams to subtract, set by tare. Forgotten when the touch ends.
    offset: f32,
    phase: Phase,
}

impl App {
    pub fn new() -> Self {
        Self {
            snapshot: Snapshot::default(),
            loaded: false,
            unit: Unit::Grams,
            help: false,
            toast: None,
            offset: 0.0,
            phase: Phase::Idle,
        }
    }

    pub fn available(&self) -> bool {
        self.snapshot.unavailable.is_none()
    }

    /// The weight on the trackpad in grams, without the tare.
    pub fn grams(&self) -> f32 {
        if self.snapshot.touching {
            (self.snapshot.grams - self.offset).max(0.0)
        } else {
            0.0
        }
    }

    /// The readout text, e.g. `124.5` or `4.39`.
    pub fn readout(&self) -> String {
        match self.unit {
            Unit::Grams => format!("{:.1}", self.grams()),
            Unit::Ounces => format!("{:.2}", self.grams() / GRAMS_PER_OUNCE),
        }
    }

    pub fn unit_label(&self) -> &'static str {
        match self.unit {
            Unit::Grams => "g",
            Unit::Ounces => "oz",
        }
    }

    pub fn zeroed(&self) -> bool {
        self.phase == Phase::Zeroed
    }

    /// Zeroed with nothing on the scale yet.
    pub fn ready(&self) -> bool {
        self.zeroed() && self.grams() < WEIGHT_MIN
    }

    pub fn step(&self) -> Step {
        match self.phase {
            Phase::Idle => Step::Rest,
            Phase::Settling { .. } => Step::Zeroing,
            Phase::Zeroed if !self.ready() && self.snapshot.stable => Step::Done,
            Phase::Zeroed => Step::Place,
        }
    }

    /// Take a new reading: start, continue or reset the auto-zero.
    pub(crate) fn observe(&mut self, snapshot: Snapshot, now: Instant) {
        self.snapshot = snapshot;
        self.loaded = true;
        if !self.snapshot.touching {
            self.offset = 0.0;
            self.phase = Phase::Idle;
            return;
        }
        let grams = self.snapshot.grams;
        match self.phase {
            Phase::Idle => {
                self.phase = Phase::Settling {
                    anchor: grams,
                    since: now,
                }
            }
            Phase::Settling { anchor, .. } if (grams - anchor).abs() > ZERO_SPREAD => {
                self.phase = Phase::Settling {
                    anchor: grams,
                    since: now,
                }
            }
            _ => {}
        }
        self.advance(now);
    }

    /// Zero once the finger has been still long enough.
    pub(crate) fn advance(&mut self, now: Instant) {
        if let Phase::Settling { since, .. } = self.phase
            && now.duration_since(since) >= ZERO_AFTER
        {
            self.offset = self.snapshot.grams;
            self.phase = Phase::Zeroed;
        }
    }

    fn tare(&mut self) {
        if !self.available() {
            return;
        }
        if self.snapshot.touching {
            self.offset = self.snapshot.grams;
            self.phase = Phase::Zeroed;
            self.toast = Some(Toast::ok("Zeroed."));
        } else {
            self.toast = Some(Toast::error(
                "Nothing to zero. Rest a finger on the trackpad first.",
            ));
        }
    }

    fn toggle_unit(&mut self) {
        self.unit = match self.unit {
            Unit::Grams => Unit::Ounces,
            Unit::Ounces => Unit::Grams,
        };
    }
}

impl telmo_kit::App for App {
    type Event = Event;

    fn draw(&self, frame: &mut ratatui::Frame) {
        crate::ui::draw(self, frame);
    }

    fn key(&mut self, key: KeyEvent) -> Flow {
        if self.help {
            self.help = false;
            return Flow::Continue;
        }
        match key.code {
            KeyCode::Char(' ') => self.tare(),
            KeyCode::Char('u') => self.toggle_unit(),
            KeyCode::Char('?') => self.help = true,
            KeyCode::Esc | KeyCode::Char('q') => return Flow::Quit,
            _ => {}
        }
        Flow::Continue
    }

    fn event(&mut self, event: Event) -> Flow {
        let Event::Snapshot(snapshot) = event;
        self.observe(snapshot, Instant::now());
        Flow::Continue
    }

    fn tick(&mut self) -> Flow {
        if self.toast.as_ref().is_some_and(Toast::expired) {
            self.toast = None;
        }
        self.advance(Instant::now());
        Flow::Continue
    }

    fn animating(&self) -> bool {
        self.toast.is_some() || matches!(self.phase, Phase::Settling { .. })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::mock;

    fn at(t0: Instant, ms: u64) -> Instant {
        t0 + Duration::from_millis(ms)
    }

    #[test]
    fn touch_steady_zero_weight_stable_lift() {
        let t0 = Instant::now();
        let mut app = App::new();
        assert_eq!(app.step(), Step::Rest);

        app.observe(mock::snapshot(22.0, false), at(t0, 0));
        assert_eq!(app.step(), Step::Zeroing);
        app.observe(mock::snapshot(22.2, false), at(t0, 300));
        assert_eq!(app.step(), Step::Zeroing);
        app.observe(mock::snapshot(22.1, true), at(t0, 650));
        assert!(app.ready());
        assert_eq!(app.step(), Step::Place);
        assert_eq!(app.readout(), "0.0");

        app.observe(mock::snapshot(100.0, false), at(t0, 900));
        assert_eq!(app.step(), Step::Place);
        app.observe(mock::snapshot(146.6, true), at(t0, 2000));
        assert_eq!(app.step(), Step::Done);
        assert_eq!(app.readout(), "124.5");

        app.observe(mock::idle(), at(t0, 2100));
        assert_eq!(app.step(), Step::Rest);
        assert!(!app.zeroed());
        app.observe(mock::snapshot(30.0, false), at(t0, 2200));
        assert_eq!(app.step(), Step::Zeroing);
        assert_eq!(app.readout(), "30.0");
    }

    #[test]
    fn movement_restarts_the_wait() {
        let t0 = Instant::now();
        let mut app = App::new();
        app.observe(mock::snapshot(20.0, false), at(t0, 0));
        app.observe(mock::snapshot(25.0, false), at(t0, 500));
        app.observe(mock::snapshot(25.1, false), at(t0, 1000));
        assert_eq!(app.step(), Step::Zeroing);
        app.observe(mock::snapshot(25.1, false), at(t0, 1100));
        assert_eq!(app.step(), Step::Place);
    }

    #[test]
    fn tick_zeroes_without_new_readings() {
        let t0 = Instant::now();
        let mut app = App::new();
        app.observe(mock::snapshot(20.0, false), t0);
        app.advance(at(t0, 700));
        assert!(app.zeroed());
    }

    #[test]
    fn zero_is_kept_while_weight_changes() {
        let t0 = Instant::now();
        let mut app = App::new();
        app.observe(mock::snapshot(20.0, false), t0);
        app.advance(at(t0, 700));
        app.observe(mock::snapshot(60.0, false), at(t0, 800));
        app.observe(mock::snapshot(20.0, false), at(t0, 900));
        assert!(app.ready());
    }
}
