//! State and key handling. No drawing here.

use crate::backend::{Cmd, Event};
use crate::model::{Device, Direction, Snapshot, Target};
use ratatui::crossterm::event::KeyCode;
use std::{cell::RefCell, rc::Rc};
use telmo_kit::{Flow, runtime::KeyEvent, widgets::Toast};
use tokio::sync::mpsc::UnboundedSender;

const STEP: f32 = 0.05;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Pane {
    Output,
    Input,
    Playing,
}

pub enum Dialog {
    Help,
    /// Move a stream to another output. `selected` indexes the outputs.
    Route {
        stream: String,
        selected: usize,
    },
    /// Pick a Bluetooth profile. `selected` indexes the device's profiles.
    Profile {
        device: String,
        selected: usize,
    },
    /// macOS has no profile switch; the dialog only explains why.
    ProfileInfo {
        device: String,
    },
}

pub struct App {
    pub snapshot: Snapshot,
    pub pane: Pane,
    pub selected: [usize; 3],
    pub dialog: Option<Dialog>,
    pub toast: Option<Toast>,
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
            pane: Pane::Output,
            selected: [0; 3],
            dialog: None,
            toast: None,
            cmds,
            last,
        }
    }

    pub fn panes(&self) -> Vec<Pane> {
        let mut panes = vec![Pane::Output, Pane::Input];
        if self.snapshot.caps.per_app {
            panes.push(Pane::Playing);
        }
        panes
    }

    pub fn devices(&self, pane: Pane) -> &[Device] {
        match pane {
            Pane::Input => &self.snapshot.inputs,
            _ => &self.snapshot.outputs,
        }
    }

    fn len(&self, pane: Pane) -> usize {
        match pane {
            Pane::Playing => self.snapshot.streams.len(),
            _ => self.devices(pane).len(),
        }
    }

    pub fn selected(&self, pane: Pane) -> usize {
        self.selected[pane as usize]
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
        let len = self.len(self.pane);
        let current = self.selected(self.pane);
        self.selected[self.pane as usize] = current
            .saturating_add_signed(delta)
            .min(len.saturating_sub(1));
    }

    fn next_pane(&mut self) {
        let panes = self.panes();
        let at = panes.iter().position(|p| *p == self.pane).unwrap_or(0);
        self.pane = panes[(at + 1) % panes.len()];
    }

    fn target(&self) -> Option<Target> {
        let i = self.selected(self.pane);
        match self.pane {
            Pane::Output => self
                .snapshot
                .outputs
                .get(i)
                .map(|d| Target::Device(Direction::Output, d.id.clone())),
            Pane::Input => self
                .snapshot
                .inputs
                .get(i)
                .map(|d| Target::Device(Direction::Input, d.id.clone())),
            Pane::Playing => self
                .snapshot
                .streams
                .get(i)
                .map(|s| Target::Stream(s.id.clone())),
        }
    }

    fn change_volume(&mut self, direction: f32) {
        let i = self.selected(self.pane);
        let (current, name) = match self.pane {
            Pane::Playing => match self.snapshot.streams.get(i) {
                Some(s) => (Some(s.volume), s.app.clone()),
                None => return,
            },
            _ => match self.devices(self.pane).get(i) {
                Some(d) => (d.volume, d.name.clone()),
                None => return,
            },
        };
        let Some(current) = current else {
            self.error(format!("{name} has a fixed volume. Use its own controls."));
            return;
        };
        let max = if current > 1.0 { 1.5 } else { 1.0 };
        let volume = (((current + direction * STEP) * 100.0).round() / 100.0).clamp(0.0, max);
        let Some(target) = self.target() else { return };
        match &target {
            Target::Stream(_) => self.snapshot.streams[i].volume = volume,
            Target::Device(..) => self.device_mut(i).volume = Some(volume),
        }
        self.send(Cmd::SetVolume(target, volume));
    }

    fn device_mut(&mut self, i: usize) -> &mut Device {
        match self.pane {
            Pane::Input => &mut self.snapshot.inputs[i],
            _ => &mut self.snapshot.outputs[i],
        }
    }

    fn toggle_mute(&mut self) {
        let i = self.selected(self.pane);
        let Some(target) = self.target() else { return };
        let muted = match &target {
            Target::Stream(_) => &mut self.snapshot.streams[i].muted,
            Target::Device(..) => &mut self.device_mut(i).muted,
        };
        *muted = !*muted;
        let muted = *muted;
        self.send(Cmd::SetMute(target, muted));
    }

    fn set_default(&mut self) {
        let direction = match self.pane {
            Pane::Output => Direction::Output,
            Pane::Input => Direction::Input,
            Pane::Playing => return,
        };
        let i = self.selected(self.pane);
        let Some(id) = self.devices(self.pane).get(i).map(|d| d.id.clone()) else {
            return;
        };
        for (n, d) in self.snapshot_devices(direction).iter_mut().enumerate() {
            d.default = n == i;
        }
        self.send(Cmd::SetDefault(direction, id));
    }

    fn snapshot_devices(&mut self, direction: Direction) -> &mut Vec<Device> {
        match direction {
            Direction::Output => &mut self.snapshot.outputs,
            Direction::Input => &mut self.snapshot.inputs,
        }
    }

    fn open_route(&mut self) {
        if self.pane != Pane::Playing {
            return;
        }
        let Some(stream) = self.snapshot.streams.get(self.selected(Pane::Playing)) else {
            return;
        };
        let current = stream.device.clone().or_else(|| self.default_output());
        let selected = self
            .snapshot
            .outputs
            .iter()
            .position(|d| Some(&d.id) == current.as_ref())
            .unwrap_or(0);
        self.dialog = Some(Dialog::Route {
            stream: stream.id.clone(),
            selected,
        });
    }

    pub fn default_output(&self) -> Option<String> {
        self.snapshot
            .outputs
            .iter()
            .find(|d| d.default)
            .map(|d| d.id.clone())
    }

    fn open_profile(&mut self) {
        if self.pane != Pane::Output {
            return;
        }
        let Some(device) = self.snapshot.outputs.get(self.selected(Pane::Output)) else {
            return;
        };
        if !device.bluetooth {
            let message = format!(
                "{} is not a Bluetooth device, so it has no mode to switch.",
                device.name
            );
            self.error(message);
        } else if !self.snapshot.caps.profiles {
            self.dialog = Some(Dialog::ProfileInfo {
                device: device.id.clone(),
            });
        } else if device.profiles.is_empty() {
            let message = format!(
                "{} reports no modes. Reconnect it and try again.",
                device.name
            );
            self.error(message);
        } else {
            // Start on the mode you would switch to.
            let selected = device.profiles.iter().position(|p| !p.active).unwrap_or(0);
            self.dialog = Some(Dialog::Profile {
                device: device.id.clone(),
                selected,
            });
        }
    }

    fn dialog_key(&mut self, key: KeyEvent) {
        let delta = match key.code {
            KeyCode::Char('j') | KeyCode::Down => 1,
            KeyCode::Char('k') | KeyCode::Up => -1,
            _ => 0,
        };
        let enter = key.code == KeyCode::Enter;
        let count = match &self.dialog {
            Some(Dialog::Route { .. }) => self.snapshot.outputs.len(),
            Some(Dialog::Profile { device, .. }) => self
                .snapshot
                .outputs
                .iter()
                .find(|d| &d.id == device)
                .map_or(0, |d| d.profiles.len()),
            _ => 0,
        };
        if let Some(Dialog::Route { selected, .. } | Dialog::Profile { selected, .. }) =
            &mut self.dialog
        {
            *selected = selected
                .saturating_add_signed(delta)
                .min(count.saturating_sub(1));
        }
        if enter {
            self.confirm_dialog();
        }
    }

    fn confirm_dialog(&mut self) {
        match self.dialog.take() {
            Some(Dialog::Route { stream, selected }) => {
                let Some(device) = self.snapshot.outputs.get(selected).map(|d| d.id.clone()) else {
                    return;
                };
                if let Some(s) = self.snapshot.streams.iter_mut().find(|s| s.id == stream) {
                    s.device = Some(device.clone());
                }
                self.send(Cmd::MoveStream { stream, device });
            }
            Some(Dialog::Profile { device, selected }) => {
                let Some(d) = self.snapshot.outputs.iter_mut().find(|d| d.id == device) else {
                    return;
                };
                let Some(profile) = d.profiles.get(selected).map(|p| p.id.clone()) else {
                    return;
                };
                for p in &mut d.profiles {
                    p.active = p.id == profile;
                }
                self.send(Cmd::SetProfile { device, profile });
            }
            other => self.dialog = other,
        }
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
            KeyCode::Char('h') | KeyCode::Left => self.change_volume(-1.0),
            KeyCode::Char('l') | KeyCode::Right => self.change_volume(1.0),
            KeyCode::Char('m') => self.toggle_mute(),
            KeyCode::Enter => self.set_default(),
            KeyCode::Char('o') => self.open_route(),
            KeyCode::Char('P') => self.open_profile(),
            _ => {}
        }
        Flow::Continue
    }

    fn event(&mut self, event: Event) -> Flow {
        match event {
            Event::Snapshot(snapshot) => {
                *self.last.borrow_mut() = Some(snapshot.clone());
                self.snapshot = snapshot;
                if !self.panes().contains(&self.pane) {
                    self.pane = Pane::Output;
                }
                for pane in self.panes() {
                    let last = self.len(pane).saturating_sub(1);
                    self.selected[pane as usize] = self.selected(pane).min(last);
                }
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
