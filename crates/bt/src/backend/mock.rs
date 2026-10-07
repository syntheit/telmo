//! Fake system for `--mock`, UI work and tests.

use super::{Cmd, Event, Rx, Tx};
use crate::model::{Adapter, Battery, Device, Kind, PairPrompt, Snapshot};
use std::{future::pending, time::Duration};
use tokio::{
    task::JoinHandle,
    time::{sleep, timeout},
};

const ACTION_DELAY: Duration = Duration::from_millis(700);
const DISCOVERY_STEP: Duration = Duration::from_millis(500);
const PASSKEY_TIMEOUT: Duration = Duration::from_secs(3);

const UNREACHABLE: &str = "Sony WH-1000XM5 didn't respond. Make sure it's on and nearby.";

fn device(id: &str, name: &str, kind: Kind) -> Device {
    Device {
        id: id.into(),
        name: name.into(),
        kind,
        paired: true,
        connected: false,
        auto_connect: Some(true),
        battery: None,
        rssi: None,
    }
}

fn connected(mut device: Device, battery: Battery) -> Device {
    device.connected = true;
    device.battery = Some(battery);
    device
}

fn nearby(id: &str, name: &str, kind: Kind, rssi: i16) -> Device {
    Device {
        paired: false,
        auto_connect: Some(false),
        rssi: Some(rssi),
        ..device(id, name, kind)
    }
}

pub fn paired_devices() -> Vec<Device> {
    let buds = Battery::Buds {
        left: Some(82),
        right: Some(80),
        case: Some(45),
    };
    vec![
        connected(
            device("4C:87:5D:1A:0F:22", "AirPods Pro", Kind::Headphones),
            buds,
        ),
        connected(
            device("E4:5F:01:2B:77:10", "Keychron K3", Kind::Keyboard),
            Battery::Single(61),
        ),
        device("D2:11:80:4A:9C:03", "MX Master 3S", Kind::Mouse),
        device("88:C9:E8:51:30:BE", "Sony WH-1000XM5", Kind::Headphones),
        device(
            "98:7A:14:0D:6E:55",
            "Xbox Wireless Controller",
            Kind::Gamepad,
        ),
    ]
}

/// Found one by one while scanning.
pub fn nearby_devices() -> Vec<Device> {
    vec![
        nearby("F0:3A:5B:22:C1:08", "Keychron Q1", Kind::Keyboard, -48),
        nearby(
            "24:E1:24:7F:90:A2",
            "Galaxy Buds2 Pro",
            Kind::Headphones,
            -55,
        ),
        nearby("04:52:C7:1E:3D:F9", "Bose QC45", Kind::Headphones, -62),
        nearby("7C:11:BE:90:25:44", "", Kind::Other, -80),
    ]
}

pub fn snapshot() -> Snapshot {
    Snapshot {
        adapter: Some(Adapter {
            name: "Mock adapter".into(),
            powered: true,
            discovering: false,
        }),
        devices: paired_devices(),
    }
}

struct Mock {
    snapshot: Snapshot,
    undiscovered: Vec<Device>,
    events: Tx,
}

pub fn spawn(cmds: Rx, events: Tx) -> JoinHandle<()> {
    tokio::spawn(run(cmds, events))
}

async fn run(mut cmds: Rx, events: Tx) {
    let mut mock = Mock {
        snapshot: snapshot(),
        undiscovered: Vec::new(),
        events,
    };
    mock.publish();
    loop {
        let step = async {
            if mock.undiscovered.is_empty() {
                pending().await
            } else {
                sleep(DISCOVERY_STEP).await
            }
        };
        tokio::select! {
            cmd = cmds.recv() => match cmd {
                Some(cmd) => mock.handle(cmd, &mut cmds).await,
                None => return,
            },
            () = step => mock.discover_next(),
        }
    }
}

impl Mock {
    fn publish(&self) {
        let _ = self.events.send(Event::Snapshot(self.snapshot.clone()));
    }

    fn done(&self, target: &str, result: Result<String, String>) {
        let target = target.to_string();
        let _ = self.events.send(Event::Done { target, result });
    }

    fn adapter(&mut self) -> Option<&mut Adapter> {
        self.snapshot.adapter.as_mut()
    }

    fn device(&mut self, id: &str) -> Option<&mut Device> {
        self.snapshot.devices.iter_mut().find(|d| d.id == id)
    }

    fn name_of(&mut self, id: &str) -> String {
        self.device(id).map(|d| d.name.clone()).unwrap_or_default()
    }

    fn discover_next(&mut self) {
        let found = self.undiscovered.remove(0);
        self.snapshot.devices.push(found);
        self.publish();
    }

    async fn handle(&mut self, cmd: Cmd, cmds: &mut Rx) {
        match cmd {
            Cmd::SetPower(on) => self.set_power(on).await,
            Cmd::Connect(id) => self.connect(&id).await,
            Cmd::Disconnect(id) => self.disconnect(&id).await,
            Cmd::StartScan => self.set_scanning(true),
            Cmd::StopScan => self.set_scanning(false),
            Cmd::Pair(id) => self.pair(&id, cmds).await,
            Cmd::PairReply(_) => {}
            Cmd::SetAutoConnect(id, on) => self.set_auto_connect(&id, on).await,
            Cmd::Forget(id) => self.forget(&id).await,
            Cmd::Rename(id, name) => self.rename(&id, name).await,
        }
    }

    async fn set_power(&mut self, on: bool) {
        sleep(ACTION_DELAY).await;
        self.undiscovered.clear();
        if let Some(adapter) = self.adapter() {
            adapter.powered = on;
            adapter.discovering = false;
        }
        if !on {
            self.snapshot.devices.retain(|d| d.paired);
            for device in &mut self.snapshot.devices {
                device.connected = false;
            }
        }
        self.publish();
        let message = if on {
            "Bluetooth is on"
        } else {
            "Bluetooth is off"
        };
        self.done("adapter", Ok(message.into()));
    }

    fn set_scanning(&mut self, on: bool) {
        if let Some(adapter) = self.adapter() {
            adapter.discovering = on;
        }
        if on {
            self.undiscovered = nearby_devices();
            self.undiscovered
                .retain(|n| !self.snapshot.devices.iter().any(|d| d.id == n.id));
        } else {
            self.undiscovered.clear();
            self.snapshot.devices.retain(|d| d.paired);
        }
        self.publish();
    }

    async fn connect(&mut self, id: &str) {
        sleep(ACTION_DELAY).await;
        let name = self.name_of(id);
        if name == "Sony WH-1000XM5" {
            return self.done(id, Err(UNREACHABLE.into()));
        }
        if let Some(device) = self.device(id) {
            device.connected = true;
        }
        self.publish();
        self.done(id, Ok(format!("Connected to {name}")));
    }

    async fn disconnect(&mut self, id: &str) {
        sleep(ACTION_DELAY).await;
        let name = self.name_of(id);
        if let Some(device) = self.device(id) {
            device.connected = false;
        }
        self.publish();
        self.done(id, Ok(format!("Disconnected {name}")));
    }

    async fn pair(&mut self, id: &str, cmds: &mut Rx) {
        sleep(ACTION_DELAY).await;
        let name = self.name_of(id);
        let prompt = match name.as_str() {
            "Keychron Q1" => Some((PairPrompt::DisplayPasskey(482913), false)),
            "Bose QC45" => Some((PairPrompt::Confirm(123456), true)),
            "" => Some((PairPrompt::EnterPin, true)),
            _ => None,
        };
        if let Some((prompt, user_must_answer)) = prompt {
            let device = id.to_string();
            let _ = self.events.send(Event::Pairing { device, prompt });
            if !wait_for_reply(cmds, user_must_answer).await {
                return self.done(id, Err(format!("Pairing with {name} was cancelled.")));
            }
        }
        if let Some(device) = self.device(id) {
            device.paired = true;
            device.auto_connect = Some(true);
            device.connected = true;
            device.rssi = None;
        }
        self.publish();
        self.done(id, Ok(format!("Paired with {name}")));
    }

    async fn set_auto_connect(&mut self, id: &str, on: bool) {
        sleep(ACTION_DELAY / 2).await;
        let name = self.name_of(id);
        if let Some(device) = self.device(id) {
            device.auto_connect = Some(on);
        }
        self.publish();
        let state = if on { "on" } else { "off" };
        self.done(id, Ok(format!("Auto-connect {state} for {name}")));
    }

    async fn forget(&mut self, id: &str) {
        sleep(ACTION_DELAY).await;
        let name = self.name_of(id);
        self.snapshot.devices.retain(|d| d.id != id);
        self.publish();
        self.done(id, Ok(format!("Forgot {name}")));
    }

    async fn rename(&mut self, id: &str, name: String) {
        sleep(ACTION_DELAY / 2).await;
        if let Some(device) = self.device(id) {
            device.name = name.clone();
        }
        self.publish();
        self.done(id, Ok(format!("Renamed to {name}")));
    }
}

/// Waits for the user's answer. A passkey shown on screen completes by itself
/// once the code is typed on the device, so it also ends after a timeout.
/// Returns false when the user rejected the pairing.
async fn wait_for_reply(cmds: &mut Rx, user_must_answer: bool) -> bool {
    let wait = async {
        loop {
            match cmds.recv().await {
                Some(Cmd::PairReply(reply)) => return reply.is_some(),
                Some(_) => {}
                None => return false,
            }
        }
    };
    if user_must_answer {
        wait.await
    } else {
        timeout(PASSKEY_TIMEOUT, wait).await.unwrap_or(true)
    }
}
