//! Battery levels. IOKit answers instantly (keyboards, mice, trackpads);
//! earbuds only report through `system_profiler`, which is slow, so a
//! background thread polls it and keeps a cache.

use super::{Note, normalize_address};
use crate::model::Battery;
use objc2_core_foundation::{CFDictionary, CFNumber, CFRetained, CFString, CFType};
use objc2_io_kit::{
    IOIteratorNext, IOObjectRelease, IORegistryEntryCreateCFProperty, IOServiceGetMatchingServices,
    IOServiceMatching, kIOMainPortDefault,
};
use std::{
    collections::HashMap,
    process::Command,
    sync::{
        Arc, Mutex, PoisonError,
        atomic::{AtomicBool, Ordering},
        mpsc::Sender,
    },
    thread,
    time::{Duration, Instant},
};

const PROFILER_INTERVAL: Duration = Duration::from_secs(60);

pub struct HidBattery {
    address: Option<String>,
    product: Option<String>,
    percent: u8,
}

pub fn hid_batteries() -> Vec<HidBattery> {
    let mut found = Vec::new();
    let Some(matching) =
        (unsafe { IOServiceMatching(c"AppleDeviceManagementHIDEventService".as_ptr()) })
    else {
        return found;
    };
    let mut iterator = 0;
    let status = unsafe {
        IOServiceGetMatchingServices(
            kIOMainPortDefault,
            Some(CFRetained::cast_unchecked::<CFDictionary>(matching)),
            &mut iterator,
        )
    };
    if status != 0 {
        return found;
    }
    loop {
        let service = IOIteratorNext(iterator);
        if service == 0 {
            break;
        }
        if let Some(percent) = number_property(service, "BatteryPercent") {
            found.push(HidBattery {
                address: string_property(service, "DeviceAddress")
                    .map(|address| normalize_address(&address)),
                product: string_property(service, "Product"),
                percent: percent.clamp(0, 100) as u8,
            });
        }
        IOObjectRelease(service);
    }
    IOObjectRelease(iterator);
    found
}

fn property(service: u32, key: &str) -> Option<CFRetained<CFType>> {
    let key = CFString::from_str(key);
    unsafe { IORegistryEntryCreateCFProperty(service, Some(&key), None, 0) }
}

fn number_property(service: u32, key: &str) -> Option<i64> {
    property(service, key)?.downcast_ref::<CFNumber>()?.as_i64()
}

fn string_property(service: u32, key: &str) -> Option<String> {
    Some(
        property(service, key)?
            .downcast_ref::<CFString>()?
            .to_string(),
    )
}

/// Matches by address when IOKit reports one, otherwise by product name.
pub fn single_battery(batteries: &[HidBattery], address: &str, name: &str) -> Option<Battery> {
    let by_address = batteries
        .iter()
        .find(|b| b.address.as_deref() == Some(address));
    let by_name = batteries
        .iter()
        .find(|b| b.address.is_none() && b.product.as_deref() == Some(name));
    by_address.or(by_name).map(|b| Battery::Single(b.percent))
}

#[derive(Default)]
pub struct BudsCache {
    pub buds: Mutex<HashMap<String, Battery>>,
    /// Set by the backend; the profiler only runs while something is connected.
    pub any_connected: AtomicBool,
}

impl BudsCache {
    pub fn get(&self, address: &str) -> Option<Battery> {
        let buds = self.buds.lock().unwrap_or_else(PoisonError::into_inner);
        buds.get(address).copied()
    }
}

pub fn spawn_profiler(cache: Arc<BudsCache>, notes: Sender<Note>) {
    let _ = thread::Builder::new()
        .name("bt-battery".into())
        .spawn(move || profiler_loop(&cache, &notes));
}

fn profiler_loop(cache: &BudsCache, notes: &Sender<Note>) {
    let mut last_run: Option<Instant> = None;
    loop {
        thread::sleep(Duration::from_secs(1));
        let due = last_run.is_none_or(|at| at.elapsed() >= PROFILER_INTERVAL);
        if !due || !cache.any_connected.load(Ordering::Relaxed) {
            continue;
        }
        last_run = Some(Instant::now());
        let Some(fresh) = run_profiler() else {
            continue;
        };
        let changed = {
            let mut buds = cache.buds.lock().unwrap_or_else(PoisonError::into_inner);
            let changed = *buds != fresh;
            *buds = fresh;
            changed
        };
        if changed && notes.send(Note::Changed).is_err() {
            return;
        }
    }
}

fn run_profiler() -> Option<HashMap<String, Battery>> {
    let output = Command::new("system_profiler")
        .args(["SPBluetoothDataType", "-json"])
        .output()
        .ok()?;
    parse_profiler(&output.stdout)
}

fn parse_profiler(json: &[u8]) -> Option<HashMap<String, Battery>> {
    let root: serde_json::Value = serde_json::from_slice(json).ok()?;
    let connected = root
        .get("SPBluetoothDataType")?
        .get(0)?
        .get("device_connected")?
        .as_array()?;
    let mut buds = HashMap::new();
    for entry in connected {
        for properties in entry.as_object()?.values() {
            let address = properties.get("device_address")?.as_str()?;
            if let Some(battery) = profiler_battery(properties) {
                buds.insert(normalize_address(address), battery);
            }
        }
    }
    Some(buds)
}

fn profiler_battery(properties: &serde_json::Value) -> Option<Battery> {
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

    #[test]
    fn parses_airpods() {
        let json = br#"{"SPBluetoothDataType":[{"device_connected":[{"AirPods":{
            "device_address":"70:5A:6F:6B:67:F9","device_batteryLevelLeft":"85%",
            "device_batteryLevelRight":"80%","device_batteryLevelCase":"45%"}}]}]}"#;
        let buds = parse_profiler(json).unwrap();
        assert_eq!(
            buds["70:5A:6F:6B:67:F9"],
            Battery::Buds {
                left: Some(85),
                right: Some(80),
                case: Some(45)
            }
        );
    }
}
