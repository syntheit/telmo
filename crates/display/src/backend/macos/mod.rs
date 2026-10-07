//! macOS backend. Runs on its own thread; a second thread sits in a run loop
//! and pokes the first whenever the display setup changes, which then
//! rebuilds the whole snapshot (debounced). Nothing polls.

use super::{Cmd, Event, Rx, Tx};
use crate::model::{Display, Info, Kind, Mode, Night, Snapshot};
use std::collections::HashMap;
use std::sync::mpsc;
use std::time::Duration;

mod cf;
mod ddc;
mod private;

const DEBOUNCE: Duration = Duration::from_millis(30);
const KEYBOARD: &str = "keyboard";

enum Msg {
    Cmd(Cmd),
    Changed,
    Closed,
}

pub fn spawn(cmds: Rx, events: Tx) {
    let (tx, rx) = mpsc::channel();
    let forward = tx.clone();
    std::thread::spawn(move || forward_cmds(cmds, forward));
    let watch = tx.clone();
    std::thread::spawn(move || {
        cf::watch_displays(Box::new(move || {
            let _ = watch.send(Msg::Changed);
        }))
    });
    std::thread::spawn(move || run(rx, events));
}

fn forward_cmds(mut cmds: Rx, tx: mpsc::Sender<Msg>) {
    while let Some(cmd) = cmds.blocking_recv() {
        if tx.send(Msg::Cmd(cmd)).is_err() {
            return;
        }
    }
    let _ = tx.send(Msg::Closed);
}

fn run(rx: mpsc::Receiver<Msg>, events: Tx) {
    let mut system = System::open();
    if events.send(Event::Snapshot(system.snapshot())).is_err() {
        return;
    }
    while let Ok(msg) = rx.recv() {
        let mut next = Some(msg);
        while let Some(msg) = next {
            match msg {
                Msg::Closed => return,
                Msg::Changed => {}
                Msg::Cmd(cmd) => {
                    if let Err(text) = system.execute(cmd) {
                        let _ = events.send(Event::Failed(text));
                    }
                }
            }
            next = rx.recv_timeout(DEBOUNCE).ok();
        }
        if events.send(Event::Snapshot(system.snapshot())).is_err() {
            return;
        }
    }
}

struct System {
    services: Option<private::DisplayServices>,
    keyboard: Option<private::Keyboard>,
    blue_light: Option<private::BlueLight>,
    ddc: ddc::Ddc,
    /// What we last set over DDC; reading it back is slow and often stale.
    ddc_levels: HashMap<u32, f32>,
}

impl System {
    fn open() -> Self {
        Self {
            services: private::DisplayServices::load(),
            keyboard: private::Keyboard::open(),
            blue_light: private::BlueLight::open(),
            ddc: ddc::Ddc::open(),
            ddc_levels: HashMap::new(),
        }
    }

    fn snapshot(&mut self) -> Snapshot {
        let mut displays = Vec::new();
        let mut external = 0;
        for id in cf::active_displays() {
            let builtin = cf::is_builtin(id);
            let index = (!builtin).then(|| {
                external += 1;
                external - 1
            });
            displays.push(self.display(id, index));
        }
        if let Some(keyboard) = &self.keyboard {
            displays.push(Display {
                id: KEYBOARD.into(),
                name: "Keyboard".into(),
                kind: Kind::Keyboard,
                brightness: Some(keyboard.get()),
                info: Info::default(),
                modes: Vec::new(),
                modes_note: None,
            });
        }
        let night = self.blue_light.as_ref().and_then(|b| b.get());
        Snapshot {
            displays,
            night: night.map_or_else(Night::default, |(on, warmth)| Night {
                available: true,
                on,
                warmth,
            }),
        }
    }

    /// `external` is the position among external displays, for DDC.
    fn display(&mut self, id: u32, external: Option<usize>) -> Display {
        let builtin = external.is_none();
        let fallback = if builtin {
            "Built-in Display"
        } else {
            "External Display"
        };
        let name = private::display_name(id).unwrap_or_else(|| fallback.into());
        let current = cf::current_mode(id);
        let info = Info {
            resolution: current.map(|m| format!("{} × {}", m.pixel_width, m.pixel_height)),
            refresh: current.map(|m| m.refresh as f32).filter(|hz| *hz > 0.0),
            scale: current.map(|m| m.pixel_width as f32 / m.width.max(1) as f32),
            connection: builtin.then(|| "Built-in".to_string()),
        };
        Display {
            id: id.to_string(),
            name,
            kind: if builtin {
                Kind::Builtin
            } else {
                Kind::External
            },
            brightness: self.brightness(id, external),
            info,
            modes: modes(id),
            modes_note: None,
        }
    }

    fn brightness(&mut self, id: u32, external: Option<usize>) -> Option<f32> {
        if let Some(level) = self.services.as_ref().and_then(|s| s.get(id)) {
            return Some(level);
        }
        let n = external?;
        if let Some(level) = self.ddc_levels.get(&id) {
            return Some(*level);
        }
        let level = self.ddc.get(n)?;
        self.ddc_levels.insert(id, level);
        Some(level)
    }

    fn execute(&mut self, cmd: Cmd) -> Result<(), String> {
        match cmd {
            Cmd::Brightness { display, value } => self.set_brightness(&display, value),
            Cmd::Night { on, warmth } => {
                let light = self.blue_light.as_ref().ok_or(NO_NIGHT_SHIFT)?;
                if light.set(on, warmth) {
                    Ok(())
                } else {
                    Err("macOS did not accept the Night Shift change. Try again.".into())
                }
            }
            Cmd::Size { display, mode } => {
                let id = parse_id(&display)?;
                cf::set_mode(id, &mode)
            }
        }
    }

    fn set_brightness(&mut self, display: &str, value: f32) -> Result<(), String> {
        if display == KEYBOARD {
            let keyboard = self
                .keyboard
                .as_ref()
                .ok_or("This Mac has no keyboard backlight.")?;
            return keyboard
                .set(value)
                .then_some(())
                .ok_or_else(|| "The keyboard backlight did not respond.".into());
        }
        let id = parse_id(display)?;
        if let Some(services) = &self.services
            && services.get(id).is_some()
        {
            return services
                .set(id, value)
                .then_some(())
                .ok_or_else(|| "macOS did not accept the brightness change.".into());
        }
        let n = cf::active_displays()
            .into_iter()
            .filter(|d| !cf::is_builtin(*d))
            .position(|d| d == id)
            .ok_or("That display is gone. Reopen the app to refresh the list.")?;
        if self.ddc.set(n, value) {
            self.ddc_levels.insert(id, value);
            Ok(())
        } else {
            Err("This monitor did not accept the brightness change. Use its own buttons.".into())
        }
    }
}

const NO_NIGHT_SHIFT: &str = "Night Shift is not available on this Mac.";

fn parse_id(id: &str) -> Result<u32, String> {
    id.parse()
        .map_err(|_| "That display is gone. Reopen the app to refresh the list.".to_string())
}

/// Sizes the way System Settings offers them: HiDPI modes by how much fits
/// on screen, from "Larger text" to "More space". A display with no HiDPI
/// modes lists its plain resolutions instead.
fn modes(id: u32) -> Vec<Mode> {
    let Some(current) = cf::current_mode(id) else {
        return Vec::new();
    };
    let all: Vec<_> = cf::all_modes(id).into_iter().filter(|m| m.usable).collect();
    let hidpi = all.iter().any(|m| m.hidpi());
    let same_refresh = |m: &cf::ModeInfo| (m.refresh - current.refresh).abs() < 1.0;
    let mut chosen: Vec<_> = all
        .iter()
        .filter(|m| m.hidpi() == hidpi && same_refresh(m))
        .copied()
        .collect();
    // Notched screens offer two heights per width; keep the one shaped like
    // the current mode, as System Settings does.
    let ratio = |m: &cf::ModeInfo| m.height as f64 / m.width.max(1) as f64;
    let wanted = ratio(&current);
    chosen.sort_by(|a, b| {
        (a.width, (ratio(a) - wanted).abs())
            .partial_cmp(&(b.width, (ratio(b) - wanted).abs()))
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    chosen.dedup_by_key(|m| m.width);
    let last = chosen.len().saturating_sub(1);
    chosen
        .iter()
        .enumerate()
        .map(|(i, m)| Mode {
            id: m.id(),
            label: format!("{} × {}", m.width, m.height),
            note: note(hidpi && chosen.len() >= 3, m.is_default, i, last).into(),
            current: m.id() == current.id(),
        })
        .collect()
}

fn note(ends: bool, is_default: bool, i: usize, last: usize) -> &'static str {
    match (is_default, ends, i) {
        (true, _, _) => "Default",
        (_, true, 0) => "Larger text",
        (_, true, n) if n == last => "More space",
        _ => "",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Touches real hardware: `cargo test -p telmo-display -- --ignored`.
    /// Nudges brightness and the keyboard backlight, then puts them back.
    #[test]
    #[ignore]
    fn brightness_round_trip() {
        let mut system = System::open();
        let before = system.snapshot();
        for d in before.displays.iter().filter(|d| d.kind != Kind::External) {
            let Some(level) = d.brightness else { continue };
            let nudged = if level > 0.5 {
                level - 0.05
            } else {
                level + 0.05
            };
            system.set_brightness(&d.id, nudged).unwrap();
            let now = system.snapshot();
            let read = now
                .displays
                .iter()
                .find(|x| x.id == d.id)
                .and_then(|x| x.brightness);
            system.set_brightness(&d.id, level).unwrap();
            assert!(
                (read.unwrap() - nudged).abs() < 0.02,
                "{} read {read:?}",
                d.name
            );
        }
        let after = system.snapshot();
        for (a, b) in before.displays.iter().zip(&after.displays) {
            let (a, b) = (a.brightness.unwrap_or(0.0), b.brightness.unwrap_or(0.0));
            assert!((a - b).abs() < 0.01, "{a} vs {b}");
        }
    }
}
