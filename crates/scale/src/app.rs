//! State and key handling. No drawing here.
//!
//! There is deliberately no mouse handling: touching the trackpad would click.

use crate::backend::Event;
use crate::model::Snapshot;
use crossterm::event::{KeyCode, KeyEvent};
use telmo_kit::{Flow, widgets::Toast};

const GRAMS_PER_OUNCE: f32 = 28.349_523;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Unit {
    Grams,
    Ounces,
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

    fn tare(&mut self) {
        if !self.available() {
            return;
        }
        if self.snapshot.touching {
            self.offset = self.snapshot.grams;
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
        if !snapshot.touching {
            self.offset = 0.0;
        }
        self.snapshot = snapshot;
        self.loaded = true;
        Flow::Continue
    }

    fn tick(&mut self) -> Flow {
        if self.toast.as_ref().is_some_and(Toast::expired) {
            self.toast = None;
        }
        Flow::Continue
    }

    fn animating(&self) -> bool {
        self.toast.is_some()
    }
}
