//! CoreAudio backend. Runs on its own thread; CoreAudio listener blocks only
//! poke that thread, which rebuilds the whole snapshot (debounced).

use super::{Cmd, Event, Rx, Tx};
use crate::model::{Caps, Device, Direction, Snapshot, Target};
mod hal;

use hal::*;
use objc2_core_audio::*;
use std::sync::mpsc;
use std::time::Duration;

const DEBOUNCE: Duration = Duration::from_millis(30);

enum Msg {
    Cmd(Cmd),
    Changed,
    Closed,
}

pub fn spawn(cmds: Rx, events: Tx) {
    let (tx, rx) = mpsc::channel();
    let forward = tx.clone();
    std::thread::spawn(move || forward_cmds(cmds, forward));
    std::thread::spawn(move || run(rx, tx, events));
}

fn forward_cmds(mut cmds: Rx, tx: mpsc::Sender<Msg>) {
    while let Some(cmd) = cmds.blocking_recv() {
        if tx.send(Msg::Cmd(cmd)).is_err() {
            return;
        }
    }
    let _ = tx.send(Msg::Closed);
}

fn run(rx: mpsc::Receiver<Msg>, tx: mpsc::Sender<Msg>, events: Tx) {
    let mut listeners = Listeners::new(tx);
    let publish = |listeners: &mut Listeners| {
        listeners.watch(&device_ids());
        events.send(Event::Snapshot(snapshot())).is_ok()
    };
    if !publish(&mut listeners) {
        return;
    }
    while let Ok(msg) = rx.recv() {
        let mut next = Some(msg);
        while let Some(msg) = next {
            match msg {
                Msg::Closed => return,
                Msg::Changed => {}
                Msg::Cmd(cmd) => {
                    if let Err(text) = execute(cmd) {
                        let _ = events.send(Event::Failed(text));
                    }
                }
            }
            next = rx.recv_timeout(DEBOUNCE).ok();
        }
        if !publish(&mut listeners) {
            return;
        }
    }
}

// ---- Snapshot ----

struct Info {
    id: u32,
    uid: String,
    name: String,
    aggregate: bool,
    hidden: bool,
    bluetooth: bool,
}

impl Info {
    fn new(id: u32) -> Option<Info> {
        let transport = get::<u32>(id, global(kAudioDevicePropertyTransportType)).unwrap_or(0);
        Some(Info {
            id,
            uid: get_string(id, kAudioDevicePropertyDeviceUID)?,
            name: get_string(id, kAudioObjectPropertyName)?,
            aggregate: transport == kAudioDeviceTransportTypeAggregate,
            hidden: get::<u32>(id, global(kAudioDevicePropertyIsHidden)).unwrap_or(0) != 0,
            bluetooth: transport == kAudioDeviceTransportTypeBluetooth
                || transport == kAudioDeviceTransportTypeBluetoothLE,
        })
    }

    fn visible(&self) -> bool {
        !self.aggregate
            && !self.hidden
            && !self.name.contains("BlackHole")
            && !self.name.contains("(EQ)")
    }

    fn has_streams(&self, direction: Direction) -> bool {
        has_streams(self.id, direction)
    }
}

fn snapshot() -> Snapshot {
    let infos: Vec<Info> = device_ids().into_iter().filter_map(Info::new).collect();
    let default_out = default_device(&infos, Direction::Output);
    let default_in = default_device(&infos, Direction::Input);
    let list = |direction, default: Option<u32>| {
        infos
            .iter()
            .filter(|i| i.visible() && i.has_streams(direction))
            .map(|i| device_row(i, direction, Some(i.id) == default))
            .collect()
    };
    Snapshot {
        outputs: list(Direction::Output, default_out),
        inputs: list(Direction::Input, default_in),
        streams: Vec::new(),
        caps: Caps {
            per_app: false,
            profiles: false,
        },
    }
}

fn device_row(info: &Info, direction: Direction, default: bool) -> Device {
    let volume_addrs = volume_addrs(info.id, direction);
    let volume = (!volume_addrs.is_empty()).then(|| {
        let sum: f32 = volume_addrs
            .iter()
            .filter_map(|a| get::<f32>(info.id, *a))
            .sum();
        sum / volume_addrs.len() as f32
    });
    let muted = mute_addrs(info.id, direction)
        .iter()
        .any(|a| get::<u32>(info.id, *a).unwrap_or(0) != 0);
    Device {
        id: info.uid.clone(),
        name: info.name.clone(),
        default,
        volume,
        muted,
        bluetooth: info.bluetooth,
        profiles: Vec::new(),
    }
}

/// The device marked as default. When the EQ daemon has made an aggregate
/// "<Real Device> (EQ)" the default, the real device is shown instead.
fn default_device(infos: &[Info], direction: Direction) -> Option<u32> {
    let id = get::<u32>(SYSTEM, global(default_selector(direction)))?;
    let current = infos.iter().find(|i| i.id == id)?;
    if direction == Direction::Input || !current.aggregate || !current.name.ends_with("(EQ)") {
        return Some(id);
    }
    let real_name = current.name.trim_end_matches("(EQ)").trim_end();
    let by_name = infos.iter().find(|i| i.visible() && i.name == real_name);
    let by_members = || {
        let members = get_vec::<u32>(id, global(kAudioAggregateDevicePropertyActiveSubDeviceList));
        infos
            .iter()
            .find(|i| i.visible() && members.contains(&i.id))
    };
    Some(by_name.or_else(by_members).map_or(id, |i| i.id))
}

// ---- Commands ----

fn execute(cmd: Cmd) -> Result<(), String> {
    match cmd {
        Cmd::SetDefault(direction, uid) => set_default(direction, &uid),
        Cmd::SetVolume(Target::Device(direction, uid), value) => {
            set_volume(direction, &uid, value.clamp(0.0, 1.0))
        }
        Cmd::SetMute(Target::Device(direction, uid), muted) => set_mute(direction, &uid, muted),
        _ => Err("Not supported on macOS yet.".into()),
    }
}

fn find(uid: &str) -> Result<Info, String> {
    device_ids()
        .into_iter()
        .filter_map(Info::new)
        .find(|i| i.uid == uid)
        .ok_or_else(|| "That device isn't available any more.".to_string())
}

fn set_default(direction: Direction, uid: &str) -> Result<(), String> {
    let device = find(uid)?;
    let mut selectors = vec![default_selector(direction)];
    if direction == Direction::Output {
        selectors.push(kAudioHardwarePropertyDefaultSystemOutputDevice);
    }
    for selector in selectors {
        if !set(SYSTEM, global(selector), device.id) {
            return Err(format!("Couldn't switch to {}.", device.name));
        }
    }
    Ok(())
}

fn set_volume(direction: Direction, uid: &str, value: f32) -> Result<(), String> {
    let device = find(uid)?;
    let addrs = volume_addrs(device.id, direction);
    if addrs.is_empty() || !addrs.iter().all(|a| set(device.id, *a, value)) {
        return Err(format!("Couldn't change the volume of {}.", device.name));
    }
    Ok(())
}

fn set_mute(direction: Direction, uid: &str, muted: bool) -> Result<(), String> {
    let device = find(uid)?;
    let addrs = mute_addrs(device.id, direction);
    if addrs.is_empty() || !addrs.iter().all(|a| set(device.id, *a, u32::from(muted))) {
        return Err(format!(
            "Couldn't change the mute setting of {}.",
            device.name
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::sync::mpsc::unbounded_channel;

    fn next_snapshot(events: &mut tokio::sync::mpsc::UnboundedReceiver<Event>) -> Option<Snapshot> {
        let deadline = std::time::Instant::now() + Duration::from_secs(3);
        while std::time::Instant::now() < deadline {
            match events.try_recv() {
                Ok(Event::Snapshot(s)) => return Some(s),
                Ok(Event::Failed(text)) => panic!("{text}"),
                Err(_) => std::thread::sleep(Duration::from_millis(20)),
            }
        }
        None
    }

    /// Changes the real system volume, mute and default, then restores them.
    #[test]
    #[ignore]
    fn controls_real_devices() {
        let (cmd_tx, cmd_rx) = unbounded_channel();
        let (event_tx, mut events) = unbounded_channel();
        spawn(cmd_rx, event_tx);
        let start = next_snapshot(&mut events).expect("first snapshot");
        let device = start
            .outputs
            .iter()
            .find(|d| d.default)
            .expect("default output")
            .clone();
        let target = Target::Device(Direction::Output, device.id.clone());

        cmd_tx.send(Cmd::SetVolume(target.clone(), 0.37)).unwrap();
        let s = next_snapshot(&mut events).expect("snapshot after volume");
        let now = s.outputs.iter().find(|d| d.id == device.id).unwrap();
        assert!(
            (now.volume.unwrap() - 0.37).abs() < 0.07,
            "{:?}",
            now.volume
        );

        cmd_tx
            .send(Cmd::SetMute(target.clone(), !device.muted))
            .unwrap();
        let s = next_snapshot(&mut events).expect("snapshot after mute");
        assert_eq!(
            s.outputs.iter().find(|d| d.id == device.id).unwrap().muted,
            !device.muted
        );

        // External change (as the user's keys would do) must produce a snapshot.
        std::process::Command::new("osascript")
            .args(["-e", "set volume output volume 21"])
            .status()
            .unwrap();
        assert!(
            next_snapshot(&mut events).is_some(),
            "no snapshot after external change"
        );

        cmd_tx
            .send(Cmd::SetVolume(target.clone(), device.volume.unwrap_or(0.0)))
            .unwrap();
        cmd_tx.send(Cmd::SetMute(target, device.muted)).unwrap();
        cmd_tx
            .send(Cmd::SetDefault(Direction::Output, device.id))
            .unwrap();
        std::thread::sleep(Duration::from_millis(300));
        while let Ok(e) = events.try_recv() {
            if let Event::Failed(text) = e {
                panic!("{text}");
            }
        }
    }
}
