//! `system_profiler SPBluetoothDataType` is the source of truth for which
//! devices are connected, and the only source of earbud batteries. It takes
//! 0.5-2 s, so a worker thread runs it and keeps the latest result.

use super::{CONNECT_TIMEOUT, Note, Waker, normalize_address};
use crate::model::Battery;
use std::{
    collections::{HashMap, HashSet},
    process::Command,
    sync::{
        Arc, Mutex, PoisonError,
        mpsc::{Receiver, RecvTimeoutError, Sender, channel},
    },
    thread,
    time::{Duration, Instant},
};

const SLOW_INTERVAL: Duration = Duration::from_secs(10);
/// Used after a connect or disconnect, until the state has had time to change.
const FAST_INTERVAL: Duration = Duration::from_secs(1);

#[derive(Debug, Default, PartialEq)]
pub struct Profile {
    /// Normalized addresses listed under `device_connected`.
    connected: HashSet<String>,
    batteries: HashMap<String, Battery>,
}

/// The latest profile; empty until the first run succeeds.
#[derive(Default)]
pub struct ProfileCache(Mutex<Option<Profile>>);

impl ProfileCache {
    /// None until a profile has loaded.
    pub fn is_connected(&self, address: &str) -> Option<bool> {
        let profile = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        profile.as_ref().map(|p| p.connected.contains(address))
    }

    pub fn battery(&self, address: &str) -> Option<Battery> {
        let profile = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        profile.as_ref()?.batteries.get(address).copied()
    }

    /// Stores `fresh` and says whether it differs from what was there.
    fn replace(&self, fresh: Profile) -> bool {
        let mut profile = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        let changed = profile.as_ref() != Some(&fresh);
        *profile = Some(fresh);
        changed
    }
}

/// Starts the worker. Send on the returned channel after a connect or
/// disconnect to poll fast for a while; dropping it ends the thread.
pub fn spawn(cache: Arc<ProfileCache>, notes: Sender<Note>, waker: Arc<Waker>) -> Sender<()> {
    let (watch, requests) = channel();
    let _ = thread::Builder::new()
        .name("bt-profiler".into())
        .spawn(move || worker(&cache, &notes, &waker, &requests));
    watch
}

fn worker(cache: &ProfileCache, notes: &Sender<Note>, waker: &Waker, requests: &Receiver<()>) {
    let mut fast_until = Instant::now();
    let mut first = true;
    loop {
        let changed = run().is_some_and(|fresh| cache.replace(fresh));
        // The backend holds back its first snapshot until this first run
        // is over, even if system_profiler failed.
        let note = if first { Note::Profiled } else { Note::Changed };
        if (changed || first) && notes.send(note).is_err() {
            return;
        }
        if changed || first {
            waker.wake();
        }
        first = false;
        let wait = if Instant::now() < fast_until {
            FAST_INTERVAL
        } else {
            SLOW_INTERVAL
        };
        match requests.recv_timeout(wait) {
            Ok(()) => fast_until = Instant::now() + CONNECT_TIMEOUT + FAST_INTERVAL,
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => return,
        }
    }
}

fn run() -> Option<Profile> {
    let output = Command::new("system_profiler")
        .args(["SPBluetoothDataType", "-json"])
        .output()
        .ok()?;
    parse(&output.stdout)
}

fn parse(json: &[u8]) -> Option<Profile> {
    let root: serde_json::Value = serde_json::from_slice(json).ok()?;
    let controller = root.get("SPBluetoothDataType")?.get(0)?;
    let mut profile = Profile::default();
    // Devices missing from `device_connected` (including everything under
    // `device_not_connected`) are not connected.
    let connected = controller
        .get("device_connected")
        .and_then(|c| c.as_array());
    for entry in connected.into_iter().flatten() {
        for properties in entry.as_object().into_iter().flat_map(|e| e.values()) {
            let Some(address) = properties.get("device_address").and_then(|a| a.as_str()) else {
                continue;
            };
            let address = normalize_address(address);
            if let Some(battery) = battery(properties) {
                profile.batteries.insert(address.clone(), battery);
            }
            profile.connected.insert(address);
        }
    }
    Some(profile)
}

fn battery(properties: &serde_json::Value) -> Option<Battery> {
    let level = |key: &str| {
        let text = properties.get(key)?.as_str()?;
        text.trim().trim_end_matches('%').parse::<u8>().ok()
    };
    let (left, right, case) = (
        level("device_batteryLevelLeft"),
        level("device_batteryLevelRight"),
        level("device_batteryLevelCase"),
    );
    if left.is_some() || right.is_some() || case.is_some() {
        return Some(Battery::Buds { left, right, case });
    }
    level("device_batteryLevelMain").map(Battery::Single)
}

#[cfg(test)]
mod tests {
    use super::*;

    const FIXTURE: &[u8] = br#"{"SPBluetoothDataType":[{
        "controller_properties":{"controller_state":"attrib_on"},
        "device_connected":[{"Daniel Pods":{
            "device_address":"70-5a-6f-6b-67-f9","device_batteryLevelMain":"100%",
            "device_minorType":"Headset","device_services":"0x200000 < HFP AVRCP A2DP ACL >"}},
          {"Buds":{"device_address":"AA:BB:CC:DD:EE:01","device_batteryLevelLeft":"85%",
            "device_batteryLevelRight":"80%","device_batteryLevelCase":"45%"}}],
        "device_not_connected":[{"Keyboard":{"device_address":"AA:BB:CC:DD:EE:02"}}]}]}"#;

    #[test]
    fn splits_connected_from_not_connected() {
        let profile = parse(FIXTURE).unwrap();
        assert!(profile.connected.contains("70:5A:6F:6B:67:F9"));
        assert!(profile.connected.contains("AA:BB:CC:DD:EE:01"));
        assert!(!profile.connected.contains("AA:BB:CC:DD:EE:02"));
    }

    #[test]
    fn no_connected_devices() {
        let json = br#"{"SPBluetoothDataType":[{"controller_properties":{}}]}"#;
        assert_eq!(parse(json).unwrap(), Profile::default());
    }

    #[test]
    fn rejects_garbage() {
        assert!(parse(b"nope").is_none());
    }

    #[test]
    fn parses_batteries() {
        let batteries = parse(FIXTURE).unwrap().batteries;
        assert_eq!(
            batteries["AA:BB:CC:DD:EE:01"],
            Battery::Buds {
                left: Some(85),
                right: Some(80),
                case: Some(45)
            }
        );
        assert_eq!(batteries["70:5A:6F:6B:67:F9"], Battery::Single(100));
    }
}
