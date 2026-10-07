//! Fake system for `--mock`, UI work and tests: a believable MacBook.

use super::{AWAKE, Cmd, Event, MODE, Rx, Tx};
use crate::model::{
    AwakeChoice, Battery, ChargeState, EnergyUser, Health, KeepAwake, ModeSetting, PowerMode,
    Snapshot,
};
use std::time::Duration;
use tokio::{task::JoinHandle, time::sleep};

const ACTION_DELAY: Duration = Duration::from_millis(700);
const USERS_DELAY: Duration = Duration::from_millis(1200);

pub fn snapshot() -> Snapshot {
    Snapshot {
        battery: Some(Battery {
            percent: 82,
            state: ChargeState::Charging,
            minutes: Some(65),
            source: Some("96 W adapter".into()),
            health: Some(Health {
                max_capacity: Some(91),
                cycles: Some(214),
                condition: Some("normal".into()),
            }),
        }),
        mode: Some(ModeSetting {
            current: PowerMode::Balanced,
            available: vec![PowerMode::Balanced, PowerMode::Saver],
        }),
        keep_awake: KeepAwake::Off,
        lists_energy_users: true,
    }
}

pub fn energy_users() -> Vec<EnergyUser> {
    [
        ("Zen", 46.8),
        ("Spotify", 31.5),
        ("Ghostty", 12.1),
        ("Telegram", 10.3),
        ("coreaudiod", 4.6),
    ]
    .map(|(name, power)| EnergyUser {
        name: name.into(),
        power,
    })
    .to_vec()
}

pub fn spawn(cmds: Rx, events: Tx) -> JoinHandle<()> {
    tokio::spawn(run(cmds, events))
}

async fn run(mut cmds: Rx, events: Tx) {
    let mut snapshot = snapshot();
    let _ = events.send(Event::Snapshot(snapshot.clone()));
    let users = events.clone();
    tokio::spawn(async move {
        sleep(USERS_DELAY).await;
        let _ = users.send(Event::EnergyUsers(energy_users()));
    });
    while let Some(cmd) = cmds.recv().await {
        sleep(ACTION_DELAY).await;
        let (target, message) = match cmd {
            Cmd::SetMode(mode) => {
                if let Some(setting) = snapshot.mode.as_mut() {
                    setting.current = mode;
                }
                (MODE, format!("Power mode is {}.", mode.label()))
            }
            Cmd::SetKeepAwake(choice) => {
                snapshot.keep_awake = match choice {
                    AwakeChoice::Off => KeepAwake::Off,
                    AwakeChoice::Minutes(minutes_left) => KeepAwake::Timed { minutes_left },
                    AwakeChoice::Indefinite => KeepAwake::Indefinite,
                };
                (AWAKE, format!("Keep awake: {}.", choice.label()))
            }
        };
        let _ = events.send(Event::Snapshot(snapshot.clone()));
        let result = Ok(message);
        let _ = events.send(Event::Done { target, result });
    }
}
