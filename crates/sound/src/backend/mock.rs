//! Fake system for `--mock`, UI work and tests. `TELMO_MOCK_OS=mac` gives the
//! macOS variant: no per-app volume, no profiles, one fixed-volume monitor.

use super::{Cmd, Event, Rx, Tx};
use crate::model::{Caps, Device, Direction, Profile, Snapshot, Stream, Target};

pub fn spawn(mut cmds: Rx, events: Tx) {
    let mut snapshot = if std::env::var("TELMO_MOCK_OS").is_ok_and(|os| os == "mac") {
        mac()
    } else {
        linux()
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

fn device(id: &str, name: &str, default: bool, volume: Option<f32>) -> Device {
    Device {
        id: id.into(),
        name: name.into(),
        default,
        volume,
        muted: false,
        bluetooth: false,
        profiles: Vec::new(),
        playing: false,
    }
}

fn stream(id: &str, app: &str, volume: f32, muted: bool, device: Option<&str>) -> Stream {
    Stream {
        id: id.into(),
        app: app.into(),
        volume,
        muted,
        device: device.map(Into::into),
        playing: false,
    }
}

fn profile(id: &str, name: &str, active: bool) -> Profile {
    Profile {
        id: id.into(),
        name: name.into(),
        active,
    }
}

pub fn linux() -> Snapshot {
    let airpods = Device {
        bluetooth: true,
        profiles: vec![
            profile("a2dp", "Best sound (no mic)", true),
            profile("hfp", "Headset (mic on, lower quality)", false),
        ],
        ..device("airpods", "AirPods Pro", false, Some(0.45))
    };
    Snapshot {
        outputs: vec![
            Device {
                playing: true,
                ..device("builtin", "Built-in Audio", true, Some(0.7))
            },
            airpods,
            device("minispk", "Mac mini speakers", false, Some(1.0)),
            device("lg", "LG UltraFine", false, Some(1.0)),
        ],
        inputs: vec![device("snowball", "Blue Snowball", true, Some(0.8))],
        streams: vec![
            Stream {
                playing: true,
                ..stream("spotify", "Spotify", 0.8, false, Some("builtin"))
            },
            stream("zen", "Zen Browser", 1.0, false, Some("airpods")),
            stream("discord", "Discord", 0.65, true, None),
        ],
        caps: Caps {
            per_app: true,
            profiles: true,
        },
    }
}

pub fn mac() -> Snapshot {
    let airpods = Device {
        bluetooth: true,
        ..device("airpods", "AirPods Pro", false, Some(0.4))
    };
    let airpods_mic = Device {
        bluetooth: true,
        ..device("airpods-mic", "AirPods Pro", false, Some(0.5))
    };
    Snapshot {
        outputs: vec![
            Device {
                playing: true,
                ..device("speakers", "MacBook Pro Speakers", true, Some(0.62))
            },
            airpods,
            device("lg", "LG UltraFine", false, None),
        ],
        inputs: vec![
            device("mic", "MacBook Pro Microphone", true, Some(0.75)),
            airpods_mic,
        ],
        streams: Vec::new(),
        caps: Caps::default(),
    }
}

fn devices(snapshot: &mut Snapshot, direction: Direction) -> &mut Vec<Device> {
    match direction {
        Direction::Output => &mut snapshot.outputs,
        Direction::Input => &mut snapshot.inputs,
    }
}

fn missing(what: &str) -> String {
    format!("{what} is no longer available. Pick another one.")
}

fn apply(snapshot: &mut Snapshot, cmd: Cmd) -> Result<(), String> {
    match cmd {
        Cmd::SetDefault(direction, id) => {
            let list = devices(snapshot, direction);
            if !list.iter().any(|d| d.id == id) {
                return Err(missing("That device"));
            }
            for d in list {
                d.default = d.id == id;
            }
        }
        Cmd::SetVolume(target, volume) => match target {
            Target::Device(direction, id) => {
                let d = find_device(snapshot, direction, &id)?;
                if d.volume.is_none() {
                    return Err(format!("{} has a fixed volume.", d.name));
                }
                d.volume = Some(volume);
            }
            Target::Stream(id) => find_stream(snapshot, &id)?.volume = volume,
        },
        Cmd::SetMute(target, muted) => match target {
            Target::Device(direction, id) => find_device(snapshot, direction, &id)?.muted = muted,
            Target::Stream(id) => find_stream(snapshot, &id)?.muted = muted,
        },
        Cmd::MoveStream { stream, device } => {
            if !snapshot.outputs.iter().any(|d| d.id == device) {
                return Err(missing("That output"));
            }
            find_stream(snapshot, &stream)?.device = Some(device);
        }
        Cmd::SetProfile { device, profile } => {
            let d = find_device(snapshot, Direction::Output, &device)?;
            if !d.profiles.iter().any(|p| p.id == profile) {
                return Err(format!("{} has no such mode.", d.name));
            }
            for p in &mut d.profiles {
                p.active = p.id == profile;
            }
        }
    }
    Ok(())
}

fn find_device<'a>(
    snapshot: &'a mut Snapshot,
    direction: Direction,
    id: &str,
) -> Result<&'a mut Device, String> {
    devices(snapshot, direction)
        .iter_mut()
        .find(|d| d.id == id)
        .ok_or_else(|| missing("That device"))
}

fn find_stream<'a>(snapshot: &'a mut Snapshot, id: &str) -> Result<&'a mut Stream, String> {
    snapshot
        .streams
        .iter_mut()
        .find(|s| s.id == id)
        .ok_or_else(|| missing("That app"))
}
