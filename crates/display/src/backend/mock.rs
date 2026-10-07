//! Fake system for `--mock`, UI work and tests. `TELMO_MOCK_OS=linux` gives
//! the Hyprland variant: scale choices, one monitor with no brightness control.

use super::{Cmd, Event, Rx, Tx};
use crate::model::{Display, Info, Kind, Mode, Night, Snapshot};

pub fn spawn(mut cmds: Rx, events: Tx) {
    let mut snapshot = if std::env::var("TELMO_MOCK_OS").is_ok_and(|os| os == "linux") {
        linux()
    } else {
        mac()
    };
    tokio::spawn(async move {
        let _ = events.send(Event::Snapshot(snapshot.clone()));
        while let Some(cmd) = cmds.recv().await {
            let event = match apply(&mut snapshot, cmd) {
                Ok(()) => Event::Snapshot(snapshot.clone()),
                Err(message) => Event::Failed(message),
            };
            if events.send(event).is_err() {
                break;
            }
        }
    });
}

fn mode(id: &str, label: &str, note: &str, current: bool) -> Mode {
    Mode {
        id: id.into(),
        label: label.into(),
        note: note.into(),
        current,
    }
}

fn display(id: &str, name: &str, kind: Kind, brightness: Option<f32>) -> Display {
    Display {
        id: id.into(),
        name: name.into(),
        kind,
        brightness,
        info: Info::default(),
        modes: Vec::new(),
        modes_note: None,
    }
}

pub fn mac() -> Snapshot {
    let builtin = Display {
        info: Info {
            resolution: Some("2880 × 1864".into()),
            refresh: Some(60.0),
            scale: Some(2.0),
            connection: Some("Built-in".into()),
        },
        modes: vec![
            mode("1024x640", "1024 × 640", "Larger text", false),
            mode("1280x800", "1280 × 800", "", false),
            mode("1440x900", "1440 × 900", "Default", true),
            mode("1710x1107", "1710 × 1107", "", false),
            mode("1920x1200", "1920 × 1200", "More space", false),
        ],
        ..display("builtin", "MacBook Air Built-in", Kind::Builtin, Some(0.6))
    };
    let external = Display {
        info: Info {
            resolution: Some("5120 × 2880".into()),
            refresh: Some(60.0),
            scale: Some(2.0),
            connection: Some("Thunderbolt".into()),
        },
        modes: vec![
            mode("2560x1440", "2560 × 1440", "Default", true),
            mode("3008x1692", "3008 × 1692", "More space", false),
        ],
        ..display("lg", "LG UltraFine", Kind::External, Some(0.45))
    };
    Snapshot {
        displays: vec![
            builtin,
            external,
            display("keyboard", "Keyboard", Kind::Keyboard, Some(0.3)),
        ],
        night: Night {
            available: true,
            on: false,
            warmth: 0.5,
        },
    }
}

pub fn linux() -> Snapshot {
    let note = super::SCALE_NOTE;
    let scales = |current: &str| -> Vec<Mode> {
        [
            ("1", "1920 × 1080"),
            ("1.25", "1536 × 864"),
            ("1.5", "1280 × 720"),
        ]
        .into_iter()
        .map(|(scale, logical)| mode(scale, &format!("{scale}x"), logical, scale == current))
        .collect()
    };
    let main = Display {
        info: Info {
            resolution: Some("1920 × 1080".into()),
            refresh: Some(144.0),
            scale: Some(1.0),
            connection: Some("DP-1".into()),
        },
        modes: scales("1"),
        modes_note: Some(note.into()),
        ..display("DP-1", "Dell S2722DGM", Kind::External, Some(0.45))
    };
    let side = Display {
        info: Info {
            resolution: Some("1920 × 1080".into()),
            refresh: Some(60.0),
            scale: Some(1.0),
            connection: Some("HDMI-A-1".into()),
        },
        modes: scales("1"),
        modes_note: Some(note.into()),
        ..display("HDMI-A-1", "LG 24MK430H", Kind::External, None)
    };
    Snapshot {
        displays: vec![main, side],
        night: Night {
            available: true,
            on: true,
            warmth: 0.4,
        },
    }
}

fn find<'a>(snapshot: &'a mut Snapshot, id: &str) -> Result<&'a mut Display, String> {
    snapshot
        .displays
        .iter_mut()
        .find(|d| d.id == id)
        .ok_or_else(|| "That display is gone. Reopen the app to refresh the list.".to_string())
}

fn apply(snapshot: &mut Snapshot, cmd: Cmd) -> Result<(), String> {
    match cmd {
        Cmd::Brightness { display, value } => {
            let d = find(snapshot, &display)?;
            if d.brightness.is_none() {
                return Err(format!("{} brightness is not adjustable.", d.name));
            }
            d.brightness = Some(value);
        }
        Cmd::Night { on, warmth } => {
            snapshot.night.on = on;
            snapshot.night.warmth = warmth;
        }
        Cmd::Size { display, mode } => {
            let d = find(snapshot, &display)?;
            if !d.modes.iter().any(|m| m.id == mode) {
                return Err(format!("{} has no such size.", d.name));
            }
            for m in &mut d.modes {
                m.current = m.id == mode;
            }
        }
    }
    Ok(())
}
