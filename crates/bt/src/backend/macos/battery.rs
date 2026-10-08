//! Battery levels from IOKit, which answers instantly (keyboards, mice,
//! trackpads). Earbuds only report through `system_profiler` (see `profiler`).

use super::normalize_address;
use crate::model::Battery;
use objc2_core_foundation::{CFDictionary, CFNumber, CFRetained, CFString, CFType};
use objc2_io_kit::{
    IOIteratorNext, IOObjectRelease, IORegistryEntryCreateCFProperty, IOServiceGetMatchingServices,
    IOServiceMatching, kIOMainPortDefault,
};

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
