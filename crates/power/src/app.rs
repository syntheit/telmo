//! State and key handling. No drawing here.

use crate::backend::{AWAKE, Cmd, Event, MODE};
use crate::model::{AwakeChoice, EnergyUser, KeepAwake, PowerMode, Snapshot};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use std::collections::HashMap;
use telmo_kit::{Flow, hits::Hits, widgets::Toast};
use tokio::sync::mpsc::UnboundedSender;

/// A line in the Settings pane.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Row {
    Mode,
    Awake,
}

/// What a click on a drawn area does.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Click {
    /// Index into `App::rows()`.
    Row(usize),
    /// An option in the open chooser.
    Option(usize),
    /// Same as pressing the key (key bar and dialog hints).
    Key(KeyCode),
    /// Inside a dialog: swallows the click.
    Inside,
    /// Outside the open dialog: closes it.
    Outside,
}

pub enum Dialog {
    Help,
    Details,
    Choose { row: Row, selected: usize },
}

pub struct App {
    pub snapshot: Snapshot,
    /// False until the cache or the backend has given us something to show.
    pub loaded: bool,
    /// None while `top` is still measuring.
    pub users: Option<Vec<EnergyUser>>,
    pub selected: usize,
    /// `MODE` or `AWAKE` to what is happening to that row.
    pub pending: HashMap<&'static str, &'static str>,
    pub dialog: Option<Dialog>,
    pub toast: Option<Toast>,
    pub tick: u64,
    pub hits: Hits<Click>,
    cmds: UnboundedSender<Cmd>,
    save_on_exit: bool,
}

impl App {
    pub fn new(cmds: UnboundedSender<Cmd>, cached: Option<Snapshot>) -> Self {
        Self {
            loaded: cached.is_some(),
            snapshot: cached.unwrap_or_default(),
            users: None,
            selected: 0,
            pending: HashMap::new(),
            dialog: None,
            toast: None,
            tick: 0,
            hits: Hits::default(),
            cmds,
            save_on_exit: false,
        }
    }

    /// Remember the last snapshot for the next first frame.
    pub fn save_on_exit(mut self) -> Self {
        self.save_on_exit = true;
        self
    }

    pub fn rows(&self) -> Vec<Row> {
        let mode = self.snapshot.mode.is_some().then_some(Row::Mode);
        mode.into_iter().chain([Row::Awake]).collect()
    }

    pub fn selected_row(&self) -> Row {
        let rows = self.rows();
        rows[self.selected.min(rows.len() - 1)]
    }

    pub fn loading_users(&self) -> bool {
        self.snapshot.lists_energy_users && self.users.is_none()
    }

    /// What the chooser for `row` offers, and which option is current.
    pub fn options(&self, row: Row) -> (Vec<&'static str>, usize) {
        match row {
            Row::Mode => {
                let mode = self.snapshot.mode.as_ref();
                let available = mode.map(|m| m.available.clone()).unwrap_or_default();
                let current = mode.and_then(|m| available.iter().position(|a| *a == m.current));
                let labels = available.iter().map(|m| m.label()).collect();
                (labels, current.unwrap_or(0))
            }
            Row::Awake => {
                let current = match self.snapshot.keep_awake {
                    KeepAwake::Off => AwakeChoice::Off,
                    KeepAwake::Indefinite => AwakeChoice::Indefinite,
                    KeepAwake::Timed { minutes_left } => AwakeChoice::Minutes(minutes_left),
                };
                let position = AwakeChoice::ALL.iter().position(|c| *c == current);
                let labels = AwakeChoice::ALL.iter().map(|c| c.label()).collect();
                (labels, position.unwrap_or(1))
            }
        }
    }

    fn send(&mut self, target: &'static str, label: &'static str, cmd: Cmd) {
        if self.cmds.send(cmd).is_err() {
            self.toast = Some(Toast::error(
                "Lost the power backend. Close telmo-power and open it again.",
            ));
            return;
        }
        self.pending.insert(target, label);
    }

    fn press(&mut self, code: KeyCode) -> Flow {
        telmo_kit::App::key(self, KeyEvent::new(code, KeyModifiers::NONE))
    }

    fn select_offset(&mut self, offset: isize) {
        let last = self.rows().len() - 1;
        self.selected = self.selected.saturating_add_signed(offset).min(last);
    }

    /// Clicking the selected row does what Enter does.
    fn click_row(&mut self, index: usize) -> Flow {
        if index >= self.rows().len() {
            return Flow::Continue;
        }
        if index == self.selected {
            return self.press(KeyCode::Enter);
        }
        self.selected = index;
        Flow::Continue
    }

    fn activate(&mut self) {
        let row = self.selected_row();
        let target = if row == Row::Mode { MODE } else { AWAKE };
        if self.pending.contains_key(target) {
            return;
        }
        let toggle = self
            .snapshot
            .mode
            .as_ref()
            .filter(|m| row == Row::Mode && m.is_toggle());
        match toggle {
            Some(mode) => {
                let on = mode.current != PowerMode::Saver;
                let (next, label) = if on {
                    (PowerMode::Saver, "turning on…")
                } else {
                    (PowerMode::Balanced, "turning off…")
                };
                self.send(MODE, label, Cmd::SetMode(next));
            }
            None => {
                let (_, selected) = self.options(row);
                self.dialog = Some(Dialog::Choose { row, selected });
            }
        }
    }

    fn choose(&mut self, row: Row, option: usize) {
        match row {
            Row::Mode => {
                let available = self.snapshot.mode.as_ref().map(|m| m.available.clone());
                if let Some(mode) = available.and_then(|a| a.get(option).copied()) {
                    self.send(MODE, "switching…", Cmd::SetMode(mode));
                }
            }
            Row::Awake => {
                if let Some(choice) = AwakeChoice::ALL.get(option).copied() {
                    self.send(AWAKE, "updating…", Cmd::SetKeepAwake(choice));
                }
            }
        }
    }

    fn main_key(&mut self, key: KeyEvent) -> Flow {
        match key.code {
            KeyCode::Esc | KeyCode::Char('q') => return Flow::Quit,
            KeyCode::Char('j') | KeyCode::Down => self.select_offset(1),
            KeyCode::Char('k') | KeyCode::Up => self.select_offset(-1),
            KeyCode::Enter => self.activate(),
            KeyCode::Char('i') if self.snapshot.battery.is_some() => {
                self.dialog = Some(Dialog::Details);
            }
            KeyCode::Char('?') => self.dialog = Some(Dialog::Help),
            _ => {}
        }
        Flow::Continue
    }

    /// Handles a key for the open dialog; returns the dialog to keep showing.
    fn dialog_key(&mut self, dialog: Dialog, key: KeyEvent) -> Option<Dialog> {
        match dialog {
            Dialog::Help => match key.code {
                KeyCode::Esc | KeyCode::Char('?' | 'q') | KeyCode::Enter => None,
                _ => Some(Dialog::Help),
            },
            Dialog::Details => match key.code {
                KeyCode::Esc | KeyCode::Char('i' | 'q') | KeyCode::Enter => None,
                _ => Some(Dialog::Details),
            },
            Dialog::Choose { row, selected } => self.choose_key(row, selected, key),
        }
    }

    fn choose_key(&mut self, row: Row, selected: usize, key: KeyEvent) -> Option<Dialog> {
        let count = self.options(row).0.len();
        let selected = match key.code {
            KeyCode::Esc => return None,
            KeyCode::Enter => {
                self.choose(row, selected);
                return None;
            }
            KeyCode::Char('j') | KeyCode::Down => (selected + 1).min(count.saturating_sub(1)),
            KeyCode::Char('k') | KeyCode::Up => selected.saturating_sub(1),
            _ => selected,
        };
        Some(Dialog::Choose { row, selected })
    }

    fn click_option(&mut self, option: usize) -> Flow {
        if let Some(Dialog::Choose { row, .. }) = self.dialog.take() {
            self.choose(row, option);
        }
        Flow::Continue
    }

    fn done_event(&mut self, target: &'static str, result: Result<String, String>) {
        self.pending.remove(target);
        self.toast = Some(match result {
            Ok(message) => Toast::ok(message),
            Err(message) => Toast::error(message),
        });
    }
}

impl telmo_kit::App for App {
    type Event = Event;

    fn draw(&self, frame: &mut ratatui::Frame) {
        crate::ui::draw(self, frame);
    }

    fn key(&mut self, key: KeyEvent) -> Flow {
        match self.dialog.take() {
            Some(dialog) => {
                self.dialog = self.dialog_key(dialog, key);
                Flow::Continue
            }
            None => self.main_key(key),
        }
    }

    fn mouse(&mut self, event: MouseEvent) -> Flow {
        match event.kind {
            MouseEventKind::Down(MouseButton::Left) => {
                match self.hits.at(event.column, event.row) {
                    Some(Click::Row(index)) => self.click_row(index),
                    Some(Click::Option(index)) => self.click_option(index),
                    Some(Click::Key(code)) => self.press(code),
                    Some(Click::Outside) => self.press(KeyCode::Esc),
                    Some(Click::Inside) | None => Flow::Continue,
                }
            }
            MouseEventKind::ScrollDown if self.dialog.is_none() => self.press(KeyCode::Down),
            MouseEventKind::ScrollUp if self.dialog.is_none() => self.press(KeyCode::Up),
            _ => Flow::Continue,
        }
    }

    fn event(&mut self, event: Event) -> Flow {
        match event {
            Event::Snapshot(snapshot) => {
                self.snapshot = snapshot;
                self.loaded = true;
                self.selected = self.selected.min(self.rows().len() - 1);
            }
            Event::EnergyUsers(users) => self.users = Some(users),
            Event::Done { target, result } => self.done_event(target, result),
        }
        Flow::Continue
    }

    fn tick(&mut self) -> Flow {
        self.tick += 1;
        if self.toast.as_ref().is_some_and(Toast::expired) {
            self.toast = None;
        }
        Flow::Continue
    }

    fn animating(&self) -> bool {
        self.loading_users() || !self.pending.is_empty() || self.toast.is_some()
    }
}

impl Drop for App {
    fn drop(&mut self) {
        if self.save_on_exit && self.loaded {
            telmo_kit::cache::save("power", &self.snapshot);
        }
    }
}
