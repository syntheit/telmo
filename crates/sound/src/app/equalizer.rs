//! The per-output EQ: stepping presets on the main screen, and the EQ view
//! where you pick a preset, nudge the bass and switch the EQ off.

use super::{App, Pane};
use crate::eq::{self, Choice, Config, NUDGE_LIMIT};
use crate::model::Device;
use ratatui::crossterm::event::KeyCode;
use telmo_kit::{Flow, runtime::KeyEvent, widgets::Toast};

/// The EQ screen of one output.
pub struct EqView {
    /// Id of the output being tuned.
    pub device: String,
    /// The highlighted preset, which is also the one playing.
    pub cursor: usize,
}

impl App {
    pub fn eq_output(&self, id: &str) -> Option<&Device> {
        self.snapshot.outputs.iter().find(|d| d.id == id)
    }

    /// Whether any output can be equalized.
    pub fn has_eq(&self) -> bool {
        self.snapshot.outputs.iter().any(|d| d.eq.is_some())
    }

    pub fn eq_choice(&self, device: &Device) -> Option<Choice> {
        self.eq.choice(device)
    }

    /// Changes the settings on disk and tells Telmo.app. The file is read
    /// again first, so a change Telmo.app made meanwhile (a seeded default)
    /// is kept. The mock never touches the disk.
    fn change_eq(&mut self, change: impl FnOnce(&mut Config)) {
        if !self.persist_eq {
            return change(&mut self.eq);
        }
        let mut config = Config::load();
        change(&mut config);
        if let Err(e) = config.save() {
            self.error(format!(
                "Couldn't save the EQ settings: {e}. Check the disk and try again."
            ));
        }
        self.eq = config;
        eq::reload();
    }

    /// Gives outputs we haven't seen before their default preset.
    pub(super) fn seed_eq(&mut self) {
        let outputs = self.snapshot.outputs.clone();
        if self.eq.clone().seed(&outputs) {
            self.change_eq(|config| {
                config.seed(&outputs);
            });
        }
    }

    /// The selected output, if it can be equalized; otherwise says why.
    fn selected_eq(&mut self) -> Option<(Device, Choice)> {
        let device = self
            .snapshot
            .outputs
            .get(self.selected(Pane::Output))?
            .clone();
        let Some(choice) = self.eq.choice(&device) else {
            self.error(format!("{} can't be equalized.", device.name));
            return None;
        };
        Some((device, choice))
    }

    fn apply_eq(&mut self, device: &Device, preset: usize, bass: i32) {
        self.change_eq(|config| config.set(device, preset, bass));
    }

    /// `e`: the next preset of the selected output. Turns the EQ back on,
    /// since you press it to hear the difference.
    pub(super) fn next_preset(&mut self) {
        let Some((device, choice)) = self.selected_eq() else {
            return;
        };
        let next = (choice.active + 1) % choice.presets().len();
        self.change_eq(|config| {
            config.enabled = true;
            config.set(&device, next, choice.bass);
        });
        let name = choice.presets()[next].name;
        self.toast = Some(Toast::ok(format!("EQ {name} · e for next")));
    }

    /// `E`: the EQ screen of the selected output.
    pub(super) fn open_eq(&mut self) {
        if let Some((device, choice)) = self.selected_eq() {
            self.eq_view = Some(EqView {
                device: device.id,
                cursor: choice.active,
            });
        }
    }

    /// Announces the preset that came with a new default output.
    pub(super) fn announce_default_eq(&mut self, before: Option<String>) {
        let after = self.default_output();
        if before.is_none() || before == after {
            return;
        }
        let Some(device) = after.and_then(|id| self.eq_output(&id)) else {
            return;
        };
        if let Some(choice) = self.eq.choice(device) {
            let message = format!("{} · EQ {}", device.name, choice.preset().name);
            self.toast = Some(Toast::ok(message));
        }
    }

    pub(super) fn eq_key(&mut self, key: KeyEvent) -> Flow {
        match key.code {
            KeyCode::Esc => self.eq_view = None,
            KeyCode::Char('q') => return Flow::Quit,
            KeyCode::Char('j') | KeyCode::Down => self.move_preset(1),
            KeyCode::Char('k') | KeyCode::Up => self.move_preset(-1),
            KeyCode::Char('h') | KeyCode::Left => self.set_bass(|bass| bass - 1),
            KeyCode::Char('l') | KeyCode::Right => self.set_bass(|bass| bass + 1),
            KeyCode::Char('r') => self.set_bass(|_| 0),
            KeyCode::Char('b') => self.change_eq(|config| config.enabled = !config.enabled),
            _ => {}
        }
        Flow::Continue
    }

    /// The tuned output and what it has chosen. Closes the view when the
    /// output went away.
    fn eq_target(&mut self) -> Option<(Device, Choice)> {
        let id = self.eq_view.as_ref()?.device.clone();
        let found = self
            .eq_output(&id)
            .and_then(|d| Some((d.clone(), self.eq.choice(d)?)));
        if found.is_none() {
            self.eq_view = None;
        }
        found
    }

    /// Moves the cursor and plays the preset under it right away.
    pub(super) fn move_preset(&mut self, delta: isize) {
        let Some((device, choice)) = self.eq_target() else {
            return;
        };
        let last = choice.presets().len() - 1;
        let Some(view) = &mut self.eq_view else {
            return;
        };
        view.cursor = view.cursor.saturating_add_signed(delta).min(last);
        let cursor = view.cursor;
        self.apply_eq(&device, cursor, choice.bass);
    }

    /// A click on a preset.
    pub(super) fn pick_preset(&mut self, index: usize) {
        let Some((device, choice)) = self.eq_target() else {
            return;
        };
        if index >= choice.presets().len() {
            return;
        }
        if let Some(view) = &mut self.eq_view {
            view.cursor = index;
        }
        self.apply_eq(&device, index, choice.bass);
    }

    /// Sets the bass nudge, kept within its limit.
    fn set_bass(&mut self, change: impl FnOnce(i32) -> i32) {
        let Some((device, choice)) = self.eq_target() else {
            return;
        };
        let bass = change(choice.bass).clamp(-NUDGE_LIMIT, NUDGE_LIMIT);
        self.apply_eq(&device, choice.active, bass);
    }
}
