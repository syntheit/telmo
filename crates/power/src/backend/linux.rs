//! Linux backend: UPower for the battery, power-profiles-daemon for the power
//! mode, both over the system bus. Either may be missing (a desktop has no
//! battery); the matching part of the screen is then simply absent.

use super::{AWAKE, Cmd, Event, MODE, Rx, Tx, awake};
use crate::model::{Battery, ChargeState, Health, ModeSetting, PowerMode, Snapshot};
use futures_util::StreamExt;
use std::{future::pending, time::Duration};
use tokio::{task::JoinHandle, time::interval};
use zbus::{
    Connection, fdo::PropertiesChanged, fdo::PropertiesChangedStream, fdo::PropertiesProxy, proxy,
    proxy::CacheProperties, zvariant::OwnedObjectPath,
};

/// Keep-awake time left changes without any D-Bus signal.
const POLL: Duration = Duration::from_secs(30);
/// Bursts of property changes (percentage, time, state) become one snapshot.
const DEBOUNCE: Duration = Duration::from_millis(30);

const UPOWER: &str = "org.freedesktop.UPower";
const DISPLAY_DEVICE: &str = "/org/freedesktop/UPower/devices/DisplayDevice";
const PROFILES: &str = "net.hadess.PowerProfiles";
const PROFILES_PATH: &str = "/net/hadess/PowerProfiles";

#[proxy(
    interface = "org.freedesktop.UPower",
    default_service = "org.freedesktop.UPower",
    default_path = "/org/freedesktop/UPower"
)]
trait UPower {
    fn enumerate_devices(&self) -> zbus::Result<Vec<OwnedObjectPath>>;
    #[zbus(property)]
    fn on_battery(&self) -> zbus::Result<bool>;
}

#[proxy(
    interface = "org.freedesktop.UPower.Device",
    default_service = "org.freedesktop.UPower"
)]
trait Device {
    #[zbus(property, name = "Type")]
    fn kind(&self) -> zbus::Result<u32>;
    #[zbus(property)]
    fn is_present(&self) -> zbus::Result<bool>;
    #[zbus(property)]
    fn percentage(&self) -> zbus::Result<f64>;
    #[zbus(property)]
    fn state(&self) -> zbus::Result<u32>;
    #[zbus(property)]
    fn time_to_empty(&self) -> zbus::Result<i64>;
    #[zbus(property)]
    fn time_to_full(&self) -> zbus::Result<i64>;
    #[zbus(property)]
    fn capacity(&self) -> zbus::Result<f64>;
    #[zbus(property)]
    fn charge_cycles(&self) -> zbus::Result<i32>;
}

#[proxy(
    interface = "net.hadess.PowerProfiles",
    default_service = "net.hadess.PowerProfiles",
    default_path = "/net/hadess/PowerProfiles"
)]
trait PowerProfiles {
    #[zbus(property)]
    fn active_profile(&self) -> zbus::Result<String>;
    #[zbus(property)]
    fn set_active_profile(&self, profile: &str) -> zbus::Result<()>;
    #[zbus(property)]
    fn profiles(
        &self,
    ) -> zbus::Result<Vec<std::collections::HashMap<String, zbus::zvariant::OwnedValue>>>;
}

pub fn spawn(cmds: Rx, events: Tx) -> JoinHandle<()> {
    tokio::spawn(run(cmds, events))
}

async fn run(mut cmds: Rx, events: Tx) {
    // No system bus (a container, say) is the same as no battery and no profiles.
    let conn = Connection::system().await.ok();
    let mut changes = Changes::watch(conn.as_ref()).await;
    let mut last = None;
    let mut poll = interval(POLL);
    loop {
        tokio::select! {
            _ = poll.tick() => publish(conn.as_ref(), &events, &mut last).await,
            () = changes.next() => {
                tokio::time::sleep(DEBOUNCE).await;
                publish(conn.as_ref(), &events, &mut last).await;
            }
            cmd = cmds.recv() => {
                let Some(cmd) = cmd else { return };
                let (target, result) = handle(conn.as_ref(), cmd).await;
                let _ = events.send(Event::Done { target, result });
                publish(conn.as_ref(), &events, &mut last).await;
            }
        }
    }
}

/// PropertiesChanged signals from UPower's display device and the profiles
/// daemon. A missing service just never fires.
struct Changes {
    battery: Option<PropertiesChangedStream>,
    profiles: Option<PropertiesChangedStream>,
}

impl Changes {
    async fn watch(conn: Option<&Connection>) -> Self {
        let Some(conn) = conn else {
            return Self {
                battery: None,
                profiles: None,
            };
        };
        Self {
            battery: properties(conn, UPOWER, DISPLAY_DEVICE).await,
            profiles: properties(conn, PROFILES, PROFILES_PATH).await,
        }
    }

    async fn next(&mut self) {
        let battery = next_change(self.battery.as_mut());
        let profiles = next_change(self.profiles.as_mut());
        tokio::select! {
            () = battery => {}
            () = profiles => {}
        }
    }
}

async fn properties(
    conn: &Connection,
    service: &'static str,
    path: &'static str,
) -> Option<PropertiesChangedStream> {
    let proxy = PropertiesProxy::builder(conn)
        .destination(service)
        .ok()?
        .path(path)
        .ok()?
        .build()
        .await
        .ok()?;
    proxy.receive_properties_changed().await.ok()
}

async fn next_change(stream: Option<&mut PropertiesChangedStream>) {
    match stream {
        Some(stream) => {
            let _: Option<PropertiesChanged> = stream.next().await;
        }
        None => pending().await,
    }
}

async fn publish(conn: Option<&Connection>, events: &Tx, last: &mut Option<Snapshot>) {
    let snapshot = snapshot(conn).await;
    if last.as_ref() != Some(&snapshot) {
        let _ = events.send(Event::Snapshot(snapshot.clone()));
        *last = Some(snapshot);
    }
}

async fn snapshot(conn: Option<&Connection>) -> Snapshot {
    let (battery, mode) = match conn {
        Some(conn) => (battery(conn).await, mode(conn).await),
        None => (None, None),
    };
    Snapshot {
        battery,
        mode,
        keep_awake: awake::current().await,
        lists_energy_users: false,
    }
}

async fn handle(conn: Option<&Connection>, cmd: Cmd) -> (&'static str, Result<String, String>) {
    match cmd {
        Cmd::SetMode(mode) => (MODE, set_mode(conn, mode).await),
        Cmd::SetKeepAwake(choice) => (AWAKE, awake::set(choice).await),
    }
}

async fn battery(conn: &Connection) -> Option<Battery> {
    let display = DeviceProxy::builder(conn)
        .path(DISPLAY_DEVICE)
        .ok()?
        .cache_properties(CacheProperties::No)
        .build()
        .await
        .ok()?;
    if !display.is_present().await.ok()? {
        return None;
    }
    let percent = display.percentage().await.ok()?.round().clamp(0.0, 100.0) as u8;
    let state = match display.state().await.ok()? {
        1 => ChargeState::Charging,
        2 | 3 => ChargeState::OnBattery,
        4 => ChargeState::Full,
        _ => ChargeState::NotCharging,
    };
    let seconds = match state {
        ChargeState::Charging => display.time_to_full().await.unwrap_or(0),
        ChargeState::OnBattery => display.time_to_empty().await.unwrap_or(0),
        _ => 0,
    };
    let plugged = state != ChargeState::OnBattery;
    Some(Battery {
        percent,
        state,
        minutes: (seconds > 0).then_some((seconds / 60) as u32),
        source: plugged.then(|| "AC adapter".to_string()),
        health: health(conn).await,
    })
}

/// Capacity and cycles are only on the real battery, not the display device.
async fn health(conn: &Connection) -> Option<Health> {
    let upower = UPowerProxy::new(conn).await.ok()?;
    for path in upower.enumerate_devices().await.ok()? {
        let device = DeviceProxy::builder(conn)
            .path(path)
            .ok()?
            .cache_properties(CacheProperties::No)
            .build()
            .await
            .ok()?;
        if device.kind().await.ok() != Some(2) {
            continue;
        }
        let max_capacity = device
            .capacity()
            .await
            .ok()
            .filter(|c| *c > 0.0)
            .map(|c| c.round().min(100.0) as u8);
        let cycles = device
            .charge_cycles()
            .await
            .ok()
            .and_then(|c| u32::try_from(c).ok())
            .filter(|c| *c > 0);
        let condition = max_capacity.map(|c| if c < 80 { "worn" } else { "good" });
        return Some(Health {
            max_capacity,
            cycles,
            condition: condition.map(str::to_string),
        });
    }
    None
}

fn parse_mode(name: &str) -> Option<PowerMode> {
    match name {
        "power-saver" => Some(PowerMode::Saver),
        "balanced" => Some(PowerMode::Balanced),
        "performance" => Some(PowerMode::Performance),
        _ => None,
    }
}

fn profile_name(mode: PowerMode) -> &'static str {
    match mode {
        PowerMode::Saver => "power-saver",
        PowerMode::Balanced => "balanced",
        PowerMode::Performance => "performance",
    }
}

async fn profiles_proxy(conn: &Connection) -> Option<PowerProfilesProxy<'_>> {
    PowerProfilesProxy::builder(conn)
        .cache_properties(CacheProperties::No)
        .build()
        .await
        .ok()
}

async fn mode(conn: &Connection) -> Option<ModeSetting> {
    let proxy = profiles_proxy(conn).await?;
    let current = parse_mode(&proxy.active_profile().await.ok()?)?;
    let mut available: Vec<PowerMode> = proxy
        .profiles()
        .await
        .ok()?
        .iter()
        .filter_map(|p| parse_mode(p.get("Profile")?.downcast_ref::<&str>().ok()?))
        .collect();
    available.sort_by_key(|m| *m as u8);
    Some(ModeSetting { current, available })
}

async fn set_mode(conn: Option<&Connection>, mode: PowerMode) -> Result<String, String> {
    let unavailable = "Power profiles are not available. Is power-profiles-daemon running?";
    let proxy = match conn {
        Some(conn) => profiles_proxy(conn).await.ok_or(unavailable)?,
        None => return Err(unavailable.into()),
    };
    proxy
        .set_active_profile(profile_name(mode))
        .await
        .map_err(|e| format!("Could not switch to {}: {e}", mode.label()))?;
    Ok(format!("Power mode is {}.", mode.label()))
}
