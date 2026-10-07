//! State and key handling. No drawing here.

use crate::backend::{Cmd, Event};
use crate::model::{Device, PairPrompt, Snapshot};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use std::collections::HashMap;
use telmo_kit::{Flow, hits::Hits, input::TextInput, widgets::Toast};
use tokio::sync::mpsc::UnboundedSender;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Pane {
    Connected,
    Paired,
    Nearby,
}

impl Pane {
    pub const ALL: [Pane; 3] = [Pane::Connected, Pane::Paired, Pane::Nearby];

    pub fn title(self) -> &'static str {
        match self {
            Pane::Connected => "Connected",
            Pane::Paired => "Paired",
            Pane::Nearby => "Nearby",
        }
    }
}

/// What a click on a drawn area does.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Click {
    /// Index into `App::rows()`.
    Row(usize),
    /// Same as pressing the key (key bar and dialog hints).
    Key(KeyCode),
    /// Inside a dialog: swallows the click.
    Inside,
    /// Outside the open dialog: closes it.
    Outside,
}

pub enum Dialog {
    Help,
    /// Device id.
    Details(String),
    Rename {
        id: String,
        input: TextInput,
    },
    Forget(String),
    Pairing {
        id: String,
        prompt: PairPrompt,
        input: TextInput,
    },
}

pub struct App {
    pub snapshot: Snapshot,
    /// False until the cache or the backend has given us something to show.
    pub loaded: bool,
    pub selected: Option<String>,
    /// Device id (or "adapter") to what is happening to it.
    pub pending: HashMap<String, &'static str>,
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
            selected: None,
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

    pub fn powered(&self) -> bool {
        self.snapshot.adapter.as_ref().is_some_and(|a| a.powered)
    }

    pub fn discovering(&self) -> bool {
        self.snapshot
            .adapter
            .as_ref()
            .is_some_and(|a| a.discovering)
    }

    pub fn devices(&self, pane: Pane) -> Vec<&Device> {
        let shown = |d: &&Device| match pane {
            Pane::Connected => d.connected,
            Pane::Paired => d.paired && !d.connected,
            Pane::Nearby => !d.paired && !d.connected && self.discovering(),
        };
        self.snapshot.devices.iter().filter(shown).collect()
    }

    /// Every selectable device, top to bottom.
    pub fn rows(&self) -> Vec<&Device> {
        Pane::ALL.iter().flat_map(|p| self.devices(*p)).collect()
    }

    pub fn selected_device(&self) -> Option<&Device> {
        let rows = self.rows();
        let found = rows.iter().find(|d| Some(&d.id) == self.selected.as_ref());
        found.or(rows.first()).copied()
    }

    pub fn active_pane(&self) -> Option<Pane> {
        let selected = self.selected_device()?;
        Pane::ALL
            .into_iter()
            .find(|p| self.devices(*p).iter().any(|d| d.id == selected.id))
    }

    pub fn device(&self, id: &str) -> Option<&Device> {
        self.snapshot.devices.iter().find(|d| d.id == id)
    }

    pub fn device_name(&self, id: &str) -> String {
        match self.device(id) {
            Some(d) if !d.name.is_empty() => d.name.clone(),
            Some(_) => "Unnamed device".into(),
            None => id.to_string(),
        }
    }

    fn send(&mut self, cmd: Cmd) {
        if self.cmds.send(cmd).is_err() {
            self.toast = Some(Toast::error(
                "Lost the Bluetooth backend. Close telmo-bt and open it again.",
            ));
        }
    }

    fn start(&mut self, id: &str, label: &'static str, cmd: Cmd) {
        self.pending.insert(id.to_string(), label);
        self.send(cmd);
    }

    fn select_offset(&mut self, offset: isize) {
        let rows = self.rows();
        let current = rows
            .iter()
            .position(|d| Some(&d.id) == self.selected.as_ref())
            .unwrap_or(0);
        let next = current
            .saturating_add_signed(offset)
            .min(rows.len().saturating_sub(1));
        self.selected = rows.get(next).map(|d| d.id.clone());
    }

    fn press(&mut self, code: KeyCode) -> Flow {
        telmo_kit::App::key(self, KeyEvent::new(code, KeyModifiers::NONE))
    }

    /// Clicking the selected row does what Enter does.
    fn click_row(&mut self, index: usize) -> Flow {
        let Some(id) = self.rows().get(index).map(|d| d.id.clone()) else {
            return Flow::Continue;
        };
        if self.selected_device().is_some_and(|d| d.id == id) {
            return self.press(KeyCode::Enter);
        }
        self.selected = Some(id);
        Flow::Continue
    }

    fn next_pane(&mut self) {
        let Some(active) = self.active_pane() else {
            return;
        };
        let start = Pane::ALL.iter().position(|p| *p == active).unwrap_or(0);
        let target = (1..Pane::ALL.len())
            .map(|step| Pane::ALL[(start + step) % Pane::ALL.len()])
            .find_map(|pane| self.devices(pane).first().map(|d| d.id.clone()));
        if target.is_some() {
            self.selected = target;
        }
    }

    fn activate(&mut self) {
        let Some(device) = self.selected_device().cloned() else {
            return;
        };
        if self.pending.contains_key(&device.id) {
            return;
        }
        let id = device.id;
        if device.connected {
            self.start(&id, "disconnecting…", Cmd::Disconnect(id.clone()));
        } else if device.paired {
            self.start(&id, "connecting…", Cmd::Connect(id.clone()));
        } else {
            self.start(&id, "pairing…", Cmd::Pair(id.clone()));
        }
    }

    fn toggle_scan(&mut self) {
        let cmd = if self.discovering() {
            Cmd::StopScan
        } else {
            Cmd::StartScan
        };
        self.send(cmd);
    }

    fn toggle_power(&mut self) {
        if self.snapshot.adapter.is_none() || self.pending.contains_key("adapter") {
            return;
        }
        let label = if self.powered() {
            "turning off…"
        } else {
            "turning on…"
        };
        let cmd = Cmd::SetPower(!self.powered());
        self.start("adapter", label, cmd);
    }

    fn main_key(&mut self, key: KeyEvent) -> Flow {
        if !self.powered() {
            return match key.code {
                KeyCode::Char('p') => {
                    self.toggle_power();
                    Flow::Continue
                }
                KeyCode::Esc | KeyCode::Char('q') => Flow::Quit,
                _ => Flow::Continue,
            };
        }
        match key.code {
            KeyCode::Esc | KeyCode::Char('q') => return Flow::Quit,
            KeyCode::Char('j') | KeyCode::Down => self.select_offset(1),
            KeyCode::Char('k') | KeyCode::Up => self.select_offset(-1),
            KeyCode::Tab => self.next_pane(),
            KeyCode::Enter => self.activate(),
            KeyCode::Char('s') => self.toggle_scan(),
            KeyCode::Char('p') => self.toggle_power(),
            KeyCode::Char('i') => {
                if let Some(device) = self.selected_device().filter(|d| d.paired) {
                    self.dialog = Some(Dialog::Details(device.id.clone()));
                }
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
            Dialog::Details(id) => self.details_key(id, key),
            Dialog::Rename { id, input } => self.rename_key(id, input, key),
            Dialog::Forget(id) => match key.code {
                KeyCode::Char('y') => {
                    self.start(&id, "forgetting…", Cmd::Forget(id.clone()));
                    None
                }
                KeyCode::Char('n') | KeyCode::Esc => None,
                _ => Some(Dialog::Forget(id)),
            },
            Dialog::Pairing { id, prompt, input } => self.pairing_key(id, prompt, input, key),
        }
    }

    fn details_key(&mut self, id: String, key: KeyEvent) -> Option<Dialog> {
        let device = self.device(&id).cloned()?;
        match key.code {
            KeyCode::Esc => None,
            KeyCode::Char('t') => {
                let label = if device.trusted {
                    "untrusting…"
                } else {
                    "trusting…"
                };
                self.start(&id, label, Cmd::SetTrusted(id.clone(), !device.trusted));
                None
            }
            KeyCode::Char('r') => Some(Dialog::Rename {
                id,
                input: TextInput::with_value(device.name),
            }),
            KeyCode::Char('d') => Some(Dialog::Forget(id)),
            _ => Some(Dialog::Details(id)),
        }
    }

    fn rename_key(&mut self, id: String, mut input: TextInput, key: KeyEvent) -> Option<Dialog> {
        match key.code {
            KeyCode::Esc => None,
            KeyCode::Enter => {
                let name = input.value.trim().to_string();
                if name.is_empty() {
                    return Some(Dialog::Rename { id, input });
                }
                self.start(&id, "renaming…", Cmd::Rename(id.clone(), name));
                None
            }
            _ => {
                input.handle(key);
                Some(Dialog::Rename { id, input })
            }
        }
    }

    fn pairing_key(
        &mut self,
        id: String,
        prompt: PairPrompt,
        mut input: TextInput,
        key: KeyEvent,
    ) -> Option<Dialog> {
        if key.code == KeyCode::Esc {
            self.send(Cmd::PairReply(None));
            return None;
        }
        match (&prompt, key.code) {
            (PairPrompt::Confirm(_), KeyCode::Char('y')) => {
                self.send(Cmd::PairReply(Some(String::new())));
                return None;
            }
            (PairPrompt::Confirm(_), KeyCode::Char('n')) => {
                self.send(Cmd::PairReply(None));
                return None;
            }
            (PairPrompt::EnterPin, KeyCode::Enter) if !input.value.is_empty() => {
                self.send(Cmd::PairReply(Some(input.value.clone())));
                return None;
            }
            (PairPrompt::EnterPin, _) => {
                input.handle(key);
            }
            _ => {}
        }
        Some(Dialog::Pairing { id, prompt, input })
    }

    fn snapshot_event(&mut self, snapshot: Snapshot) {
        self.snapshot = snapshot;
        self.loaded = true;
        let gone = match &self.dialog {
            Some(Dialog::Details(id) | Dialog::Forget(id) | Dialog::Rename { id, .. }) => {
                self.device(id).is_none()
            }
            _ => false,
        };
        if gone {
            self.dialog = None;
        }
    }

    fn done_event(&mut self, target: String, result: Result<String, String>) {
        self.pending.remove(&target);
        if matches!(&self.dialog, Some(Dialog::Pairing { id, .. }) if *id == target) {
            self.dialog = None;
        }
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
            Event::Snapshot(snapshot) => self.snapshot_event(snapshot),
            Event::Done { target, result } => self.done_event(target, result),
            Event::Pairing { device, prompt } => {
                self.dialog = Some(Dialog::Pairing {
                    id: device,
                    prompt,
                    input: TextInput::default(),
                });
            }
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
        self.discovering() || !self.pending.is_empty() || self.toast.is_some()
    }
}

impl Drop for App {
    fn drop(&mut self) {
        if self.save_on_exit && self.loaded {
            // Scan results and a running scan are stale by the next launch.
            let mut snapshot = self.snapshot.clone();
            snapshot.devices.retain(|d| d.paired);
            if let Some(adapter) = snapshot.adapter.as_mut() {
                adapter.discovering = false;
            }
            telmo_kit::cache::save("bt", &snapshot);
        }
    }
}
