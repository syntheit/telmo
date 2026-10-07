//! State and key handling. No drawing here.

use crate::backend::{Cmd, Event, Tx};
use crate::capture::Capture;
use crate::model::{Device, Direction, Snapshot, Target};
use crate::motion::{FPS, Motion};
use ratatui::crossterm::event::{KeyCode, KeyModifiers};
use std::{cell::RefCell, rc::Rc, time::Instant};
use telmo_kit::{
    App as _, Flow,
    hits::Hits,
    runtime::{KeyEvent, MouseButton, MouseEvent, MouseEventKind},
    widgets::Toast,
};
use tokio::sync::mpsc::UnboundedSender;

const STEP: f32 = 0.05;
/// A long pause (a stalled terminal) must not make the bars lurch.
const MAX_ELAPSED: f32 = 0.25;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Pane {
    Output,
    Input,
    Playing,
}

/// What a click or scroll can land on, recorded while drawing.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Click {
    Row(Pane, usize),
    Pane(Pane),
    /// A gauge cell: the volume (0.0-1.0) it stands for.
    Volume(Pane, usize, f32),
    Key(KeyCode),
    DialogKey(KeyCode),
    /// A row of a pick list in a dialog.
    DialogRow(usize),
    /// The dialog itself, so clicks inside it are not "outside".
    Dialog,
    Outside,
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
    pub hits: Hits<Click>,
    /// Eased bars, peak caps and colors of the visualizer.
    pub motion: Motion,
    /// The system refused to let us listen, so the band explains how to allow it.
    pub blocked: bool,
    /// Listening to the audio; exists only while something plays.
    capture: Option<Capture>,
    mock: bool,
    /// When the visualizer last moved, to step it by real elapsed time.
    moved: Option<Instant>,
    events: Tx,
    cmds: UnboundedSender<Cmd>,
    /// The last snapshot the backend sent, for the cache on exit.
    last: Rc<RefCell<Option<Snapshot>>>,
}

impl App {
    pub fn new(
        cached: Option<Snapshot>,
        cmds: UnboundedSender<Cmd>,
        last: Rc<RefCell<Option<Snapshot>>>,
        events: Tx,
        mock: bool,
    ) -> Self {
        Self {
            snapshot: cached.unwrap_or_default(),
            pane: Pane::Output,
            selected: [0; 3],
            dialog: None,
            toast: None,
            hits: Hits::default(),
            motion: Motion::new(),
            blocked: false,
            capture: None,
            mock,
            moved: None,
            events,
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

    /// The selected row's volume, or a toast when it has none.
    fn current_volume(&mut self) -> Option<f32> {
        let i = self.selected(self.pane);
        let (current, name) = match self.pane {
            Pane::Playing => {
                let s = self.snapshot.streams.get(i)?;
                (Some(s.volume), s.app.clone())
            }
            _ => {
                let d = self.devices(self.pane).get(i)?;
                (d.volume, d.name.clone())
            }
        };
        if current.is_none() {
            self.error(format!("{name} has a fixed volume. Use its own controls."));
        }
        current
    }

    fn change_volume(&mut self, direction: f32) {
        let Some(current) = self.current_volume() else {
            return;
        };
        let max = if current > 1.0 { 1.5 } else { 1.0 };
        self.set_volume((current + direction * STEP).clamp(0.0, max));
    }

    fn set_volume(&mut self, volume: f32) {
        let volume = (volume * 100.0).round() / 100.0;
        let i = self.selected(self.pane);
        let Some(target) = self.target() else { return };
        match &target {
            Target::Stream(_) => self.snapshot.streams[i].volume = volume,
            Target::Device(..) => self.device_mut(i).volume = Some(volume),
        }
        self.send(Cmd::SetVolume(target, volume));
    }

    /// Whether anything is making sound, which is when the visualizer moves.
    pub fn playing(&self) -> bool {
        self.snapshot.outputs.iter().any(|d| d.playing)
            || self.snapshot.streams.iter().any(|s| s.playing)
    }

    /// Listens only while something plays. After the system said no we don't
    /// ask again, which would only bring the permission prompt back.
    fn sync_capture(&mut self) {
        if !self.playing() {
            self.capture = None;
        } else if self.capture.is_none() && !self.blocked {
            self.capture = Some(Capture::start(self.mock, self.events.clone()));
        }
    }

    /// Moves the visualizer by the time since it last moved. Called on every
    /// spectrum frame and tick, so it runs as often as the screen redraws.
    fn animate(&mut self) {
        let now = Instant::now();
        let elapsed = self.moved.replace(now).map_or(0.0, |last| {
            now.duration_since(last).as_secs_f32().min(MAX_ELAPSED)
        });
        self.motion.step(elapsed * FPS);
    }

    #[cfg(test)]
    pub fn capture_running(&self) -> bool {
        self.capture.is_some()
    }

    fn select(&mut self, pane: Pane, index: usize) {
        if self.panes().contains(&pane) {
            self.pane = pane;
            self.selected[pane as usize] = index;
        }
    }

    fn click(&mut self, at: Option<Click>) -> Flow {
        if self.dialog.is_some() {
            return self.click_dialog(at);
        }
        let mut flow = Flow::Continue;
        match at {
            Some(Click::Row(pane, i)) if self.pane == pane && self.selected(pane) == i => {
                self.set_default()
            }
            Some(Click::Row(pane, i)) => self.select(pane, i),
            Some(Click::Pane(pane)) => self.select(pane, self.selected(pane)),
            Some(Click::Volume(pane, i, volume)) => {
                self.select(pane, i);
                self.set_volume(volume);
            }
            Some(Click::Key(code)) => flow = self.key(KeyEvent::new(code, KeyModifiers::NONE)),
            _ => {}
        }
        flow
    }

    fn click_dialog(&mut self, at: Option<Click>) -> Flow {
        match at {
            Some(Click::Outside) => self.dialog = None,
            Some(Click::DialogKey(code)) => {
                return self.key(KeyEvent::new(code, KeyModifiers::NONE));
            }
            Some(Click::DialogRow(i)) => {
                if let Some(Dialog::Route { selected, .. } | Dialog::Profile { selected, .. }) =
                    &mut self.dialog
                {
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
            Some(Click::Row(pane, i) | Click::Volume(pane, i, _)) => {
                self.select(pane, i);
                self.change_volume(-delta as f32);
            }
            _ => self.move_selection(delta),
        }
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
                if !self.panes().contains(&self.pane) {
                    self.pane = Pane::Output;
                }
                for pane in self.panes() {
                    let last = self.len(pane).saturating_sub(1);
                    self.selected[pane as usize] = self.selected(pane).min(last);
                }
                self.sync_capture();
            }
            Event::Spectrum(frame) => {
                if frame.iter().any(|b| *b > 0.0) {
                    self.blocked = false;
                }
                self.motion.set_target(&frame);
                self.animate();
            }
            Event::VisualizerBlocked => self.blocked = true,
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
        if !self.playing() {
            self.motion.silence();
        }
        self.animate();
        Flow::Continue
    }

    fn animating(&self) -> bool {
        self.toast.is_some() || !self.motion.settled()
    }
}
