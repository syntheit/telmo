//! DDC/CI over IOAVService (Apple Silicon): the same calls m1ddc and
//! MonitorControl make. Best effort; any step may fail on a given monitor or
//! dock, and the UI then says the brightness is not adjustable.

use super::cf::{self, CFRef};
use std::ffi::{c_char, c_void};
use std::time::Duration;

#[link(name = "IOKit", kind = "framework")]
unsafe extern "C" {
    fn IOServiceMatching(name: *const c_char) -> CFRef;
    fn IOServiceGetMatchingServices(main_port: u32, matching: CFRef, iterator: *mut u32) -> i32;
    fn IOIteratorNext(iterator: u32) -> u32;
    fn IOObjectRelease(object: u32) -> i32;
    fn IORegistryEntryCreateCFProperty(entry: u32, key: CFRef, alloc: CFRef, options: u32)
    -> CFRef;
    fn IOAVServiceCreateWithService(alloc: CFRef, service: u32) -> CFRef;
    fn IOAVServiceWriteI2C(
        service: CFRef,
        chip: u32,
        offset: u32,
        data: *const c_void,
        size: u32,
    ) -> i32;
    fn IOAVServiceReadI2C(
        service: CFRef,
        chip: u32,
        offset: u32,
        data: *mut c_void,
        size: u32,
    ) -> i32;
}

const CHIP: u32 = 0x37;
const OFFSET: u32 = 0x51;
const BRIGHTNESS: u8 = 0x10;
const SETTLE: Duration = Duration::from_millis(50);
const TRIES: usize = 3;

pub struct Ddc {
    /// One service per external display, in registry order.
    services: Vec<CFRef>,
    /// Brightness maximum each service reported, 100 until read.
    max: Vec<u16>,
}

impl Ddc {
    pub fn open() -> Self {
        let services = external_services();
        let max = vec![100; services.len()];
        Self { services, max }
    }

    /// Brightness 0.0-1.0 of the n-th external display.
    pub fn get(&mut self, n: usize) -> Option<f32> {
        let service = *self.services.get(n)?;
        for _ in 0..TRIES {
            if let Some((current, max)) = read_vcp(service, BRIGHTNESS) {
                if max == 0 {
                    return None;
                }
                self.max[n] = max;
                return Some((current as f32 / max as f32).clamp(0.0, 1.0));
            }
        }
        None
    }

    pub fn set(&mut self, n: usize, value: f32) -> bool {
        let Some(service) = self.services.get(n).copied() else {
            return false;
        };
        let raw = (value.clamp(0.0, 1.0) * self.max[n] as f32).round() as u16;
        (0..TRIES).any(|_| write_vcp(service, BRIGHTNESS, raw))
    }
}

/// The AV service of every display connected from outside the machine.
fn external_services() -> Vec<CFRef> {
    let mut found = Vec::new();
    // Safety: IOServiceGetMatchingServices consumes the matching dictionary;
    // every entry from the iterator is released after use.
    unsafe {
        let matching = IOServiceMatching(c"DCPAVServiceProxy".as_ptr());
        let mut iterator = 0u32;
        if matching.is_null() || IOServiceGetMatchingServices(0, matching, &mut iterator) != 0 {
            return found;
        }
        loop {
            let entry = IOIteratorNext(iterator);
            if entry == 0 {
                break;
            }
            if is_external(entry) {
                let service = IOAVServiceCreateWithService(std::ptr::null(), entry);
                if !service.is_null() {
                    found.push(service);
                }
            }
            IOObjectRelease(entry);
        }
        IOObjectRelease(iterator);
    }
    found
}

fn is_external(entry: u32) -> bool {
    let key = cf::cf_string(c"Location");
    // Safety: the property is an owned CFString or null.
    let location = unsafe { IORegistryEntryCreateCFProperty(entry, key, std::ptr::null(), 0) };
    cf::release(key);
    let external = cf::string_of(location).is_some_and(|l| l == "External");
    cf::release(location);
    external
}

/// DDC frames end with an XOR of the destination, the source and the payload.
fn checksum(payload: &[u8]) -> u8 {
    payload
        .iter()
        .fold((CHIP as u8) << 1 ^ OFFSET as u8, |a, b| a ^ b)
}

fn write_vcp(service: CFRef, code: u8, value: u16) -> bool {
    let mut frame = vec![0x84, 0x03, code, (value >> 8) as u8, value as u8];
    frame.push(checksum(&frame));
    std::thread::sleep(Duration::from_millis(10));
    // Safety: the buffer is valid for `len` bytes.
    unsafe {
        IOAVServiceWriteI2C(
            service,
            CHIP,
            OFFSET,
            frame.as_ptr().cast(),
            frame.len() as u32,
        ) == 0
    }
}

/// (current, maximum) of a VCP code.
fn read_vcp(service: CFRef, code: u8) -> Option<(u16, u16)> {
    let mut request = vec![0x82, 0x01, code];
    request.push(checksum(&request));
    std::thread::sleep(Duration::from_millis(10));
    let mut reply = [0u8; 12];
    // Safety: both buffers are valid for the sizes passed.
    let ok = unsafe {
        IOAVServiceWriteI2C(
            service,
            CHIP,
            OFFSET,
            request.as_ptr().cast(),
            request.len() as u32,
        ) == 0
            && {
                std::thread::sleep(SETTLE);
                IOAVServiceReadI2C(
                    service,
                    CHIP,
                    OFFSET,
                    reply.as_mut_ptr().cast(),
                    reply.len() as u32,
                ) == 0
            }
    };
    // Reply: source, length, 0x02, result, code, type, max hi/lo, current hi/lo.
    let valid = ok && reply[2] == 0x02 && reply[3] == 0 && reply[4] == code;
    valid.then(|| {
        let max = u16::from_be_bytes([reply[6], reply[7]]);
        let current = u16::from_be_bytes([reply[8], reply[9]]);
        (current, max)
    })
}
