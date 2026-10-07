//! State and key handling. No drawing here.

use crate::backend::{Cmd, Event};
use crate::model::{Display, Kind, Snapshot};
use ratatui::crossterm::event::{KeyCode, KeyModifiers};
use std::{cell::RefCell, rc::Rc};
use telmo_kit::{
    App as _, Flow,
    hits::Hits,
    runtime::{KeyEvent, MouseButton, MouseEvent, MouseEventKind},
    widgets::Toast,
};
use tokio::sync::mpsc::UnboundedSender;

const STEP: f32 = 0.05;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Pane {
    Displays,
    Night,
}

/// What a click or scroll can land on, recorded while drawing.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Click {
    Row(Pane, usize),
    Pane(Pane),
    /// A gauge cell: the level (0.0-1.0) it stands for.
    Gauge(Pane, usize, f32),
    Key(KeyCode),
    DialogKey(KeyCode),
    /// A row of the size list in a dialog.
    DialogRow(usize),
    /// The dialog itself, so clicks inside it are not "outside".
    Dialog,
    Outside,
}

pub enum Dialog {
    Help,
    /// Pick a size. `selected` indexes the display's modes.
    Size {
        display: String,
        selected: usize,
    },
    Details {
        display: String,
    },
}

pub struct App {
    pub snapshot: Snapshot,
    pub pane: Pane,
    pub selected: usize,
    pub dialog: Option<Dialog>,
    pub toast: Option<Toast>,
    pub hits: Hits<Click>,
    cmds: UnboundedSender<Cmd>,
    /// The last snapshot the backend sent, for the cache on exit.
    last: Rc<RefCell<Option<Snapshot>>>,
}

impl App {
    pub fn new(
        cached: Option<Snapshot>,
        cmds: UnboundedSender<Cmd>,
        last: Rc<RefCell<Option<Snapshot>>>,
    ) -> Self {
        Self {
            snapshot: cached.unwrap_or_default(),
            pane: Pane::Displays,
            selected: 0,
            dialog: None,
            toast: None,
            hits: Hits::default(),
            cmds,
            last,
        }
    }

    pub fn display(&self, id: &str) -> Option<&Display> {
        self.snapshot.displays.iter().find(|d| d.id == id)
    }

    fn current(&self) -> Option<&Display> {
        self.snapshot.displays.get(self.selected)
    }

    fn send(&self, cmd: Cmd) {
        // The receiver only closes when the backend died, and then the
        // snapshot stops updating, which the user can see.
        let _ = self.cmds.send(cmd);
    }

    fn error(&mut self, message: impl Into<String>) {
        self.toast = Some(Toast::error(message));
    }

    fn move_selection(&mut self, delta: isize) {
        if self.pane == Pane::Displays {
            let last = self.snapshot.displays.len().saturating_sub(1);
            self.selected = self.selected.saturating_add_signed(delta).min(last);
        }
    }

    fn next_pane(&mut self) {
        self.pane = match self.pane {
            Pane::Displays => Pane::Night,
            Pane::Night => Pane::Displays,
        };
    }

    fn select(&mut self, pane: Pane, index: usize) {
        self.pane = pane;
        if pane == Pane::Displays {
            self.selected = index;
        }
    }

    fn change(&mut self, direction: f32) {
        match self.pane {
            Pane::Displays => self.set_brightness(self.level() + direction * STEP),
            Pane::Night => self.set_warmth(self.snapshot.night.warmth + direction * STEP),
        }
    }

    fn level(&self) -> f32 {
        self.current().and_then(|d| d.brightness).unwrap_or(0.0)
    }

    fn set_brightness(&mut self, value: f32) {
        let value = (value.clamp(0.0, 1.0) * 100.0).round() / 100.0;
        let Some(display) = self.snapshot.displays.get_mut(self.selected) else {
            return;
        };
        if display.brightness.is_none() {
            let message = format!("{} brightness is not adjustable from here.", display.name);
            return self.error(message);
        }
        display.brightness = Some(value);
        let id = display.id.clone();
        self.send(Cmd::Brightness { display: id, value });
    }

    fn set_warmth(&mut self, warmth: f32) {
        if !self.night_available() {
            return;
        }
        let warmth = (warmth.clamp(0.0, 1.0) * 100.0).round() / 100.0;
        self.snapshot.night.warmth = warmth;
        self.snapshot.night.on = true;
        self.send(Cmd::Night { on: true, warmth });
    }

    fn toggle_night(&mut self) {
        if !self.night_available() {
            return;
        }
        let night = &mut self.snapshot.night;
        night.on = !night.on;
        let cmd = Cmd::Night {
            on: night.on,
            warmth: night.warmth,
        };
        self.send(cmd);
    }

    /// Whether Night Shift can be changed, or a toast saying it can't.
    fn night_available(&mut self) -> bool {
        if !self.snapshot.night.available {
            self.error("Night Shift is not available. On Hyprland, start hyprsunset.");
        }
        self.snapshot.night.available
    }

    fn open_size(&mut self) {
        let Some(display) = self.current() else {
            return;
        };
        if display.modes.is_empty() {
            let message = format!("{} has no other sizes.", display.name);
            return self.error(message);
        }
        let selected = display.modes.iter().position(|m| m.current).unwrap_or(0);
        self.dialog = Some(Dialog::Size {
            display: display.id.clone(),
            selected,
        });
    }

    fn open_details(&mut self) {
        let Some(display) = self.current().filter(|d| d.kind != Kind::Keyboard) else {
            return self.error("The keyboard backlight has nothing more to show.");
        };
        self.dialog = Some(Dialog::Details {
            display: display.id.clone(),
        });
    }

    /// Enter on a row: the size chooser, or the Night Shift switch.
    fn activate(&mut self) {
        match self.pane {
            Pane::Displays => self.open_size(),
            Pane::Night => self.toggle_night(),
        }
    }

    fn click(&mut self, at: Option<Click>) -> Flow {
        if self.dialog.is_some() {
            return self.click_dialog(at);
        }
        let mut flow = Flow::Continue;
        match at {
            Some(Click::Row(pane, i)) if self.pane == pane && self.selected_row(pane) == i => {
                self.activate()
            }
            Some(Click::Row(pane, i)) => self.select(pane, i),
            Some(Click::Pane(pane)) => self.select(pane, self.selected_row(pane)),
            Some(Click::Gauge(pane, i, level)) => {
                self.select(pane, i);
                match pane {
                    Pane::Displays => self.set_brightness(level),
                    Pane::Night => self.set_warmth(level),
                }
            }
            Some(Click::Key(code)) => flow = self.key(KeyEvent::new(code, KeyModifiers::NONE)),
            _ => {}
        }
        flow
    }

    fn selected_row(&self, pane: Pane) -> usize {
        match pane {
            Pane::Displays => self.selected,
            Pane::Night => 0,
        }
    }

    fn click_dialog(&mut self, at: Option<Click>) -> Flow {
        match at {
            Some(Click::Outside) => self.dialog = None,
            Some(Click::DialogKey(code)) => {
                return self.key(KeyEvent::new(code, KeyModifiers::NONE));
            }
            Some(Click::DialogRow(i)) => {
                if let Some(Dialog::Size { selected, .. }) = &mut self.dialog {
                    if *selected == i {
                        self.confirm_dialog();
                    } else {
                        *selected = i;
                    }
                }
            }
            _ => {}
        }
        Flow::Continue
    }

    fn scroll(&mut self, at: Option<Click>, delta: isize) {
        if self.dialog.is_some() {
            let code = if delta > 0 {
                KeyCode::Down
            } else {
                KeyCode::Up
            };
            return self.dialog_key(KeyEvent::new(code, KeyModifiers::NONE));
        }
        match at {
            Some(Click::Row(pane, i) | Click::Gauge(pane, i, _)) => {
                self.select(pane, i);
                self.change(-delta as f32);
            }
            _ => self.move_selection(delta),
        }
    }

    fn dialog_key(&mut self, key: KeyEvent) {
        let delta = match key.code {
            KeyCode::Char('j') | KeyCode::Down => 1,
            KeyCode::Char('k') | KeyCode::Up => -1,
            _ => 0,
        };
        let count = match &self.dialog {
            Some(Dialog::Size { display, .. }) => {
                self.display(display).map_or(0, |d| d.modes.len())
            }
            _ => 0,
        };
        if let Some(Dialog::Size { selected, .. }) = &mut self.dialog {
            *selected = selected
                .saturating_add_signed(delta)
                .min(count.saturating_sub(1));
        }
        if key.code == KeyCode::Enter {
            self.confirm_dialog();
        }
    }

    fn confirm_dialog(&mut self) {
        let Some(Dialog::Size { display, selected }) = self.dialog.take() else {
            return;
        };
        let Some(d) = self.snapshot.displays.iter_mut().find(|d| d.id == display) else {
            return;
        };
        let Some(mode) = d.modes.get(selected).map(|m| m.id.clone()) else {
            return;
        };
        for m in &mut d.modes {
            m.current = m.id == mode;
        }
        self.send(Cmd::Size { display, mode });
    }
}

impl telmo_kit::App for App {
    type Event = Event;

    fn draw(&self, frame: &mut ratatui::Frame) {
        crate::ui::draw(self, frame);
    }

    fn key(&mut self, key: KeyEvent) -> Flow {
        if self.dialog.is_some() {
            if key.code == KeyCode::Esc {
                self.dialog = None;
            } else {
                self.dialog_key(key);
            }
            return Flow::Continue;
        }
        match key.code {
            KeyCode::Esc | KeyCode::Char('q') => return Flow::Quit,
            KeyCode::Char('?') => self.dialog = Some(Dialog::Help),
            KeyCode::Tab => self.next_pane(),
            KeyCode::Char('j') | KeyCode::Down => self.move_selection(1),
            KeyCode::Char('k') | KeyCode::Up => self.move_selection(-1),
            KeyCode::Char('h') | KeyCode::Left => self.change(-1.0),
            KeyCode::Char('l') | KeyCode::Right => self.change(1.0),
            KeyCode::Char('n') => self.toggle_night(),
            KeyCode::Char('i') => self.open_details(),
            KeyCode::Enter => self.activate(),
            _ => {}
        }
        Flow::Continue
    }

    fn mouse(&mut self, event: MouseEvent) -> Flow {
        let at = self.hits.at(event.column, event.row);
        match event.kind {
            MouseEventKind::Down(MouseButton::Left) => return self.click(at),
            MouseEventKind::ScrollUp => self.scroll(at, -1),
            MouseEventKind::ScrollDown => self.scroll(at, 1),
            _ => {}
        }
        Flow::Continue
    }

    fn event(&mut self, event: Event) -> Flow {
        match event {
            Event::Snapshot(snapshot) => {
                *self.last.borrow_mut() = Some(snapshot.clone());
                self.snapshot = snapshot;
                let last = self.snapshot.displays.len().saturating_sub(1);
                self.selected = self.selected.min(last);
            }
            Event::Failed(message) => {
                // Drop optimistic edits the backend refused.
                if let Some(last) = self.last.borrow().clone() {
                    self.snapshot = last;
                }
                self.error(message);
            }
        }
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
