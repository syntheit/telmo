//! Raw CoreAudio property access, property addresses and change listeners.

use super::Msg;
use crate::model::Direction;
use block2::RcBlock;
use objc2_core_audio::*;
use objc2_core_foundation::{CFRetained, CFString};
use std::mem::{MaybeUninit, size_of};
use std::ptr::{NonNull, null};
use std::sync::mpsc;

pub const SYSTEM: u32 = kAudioObjectSystemObject as u32;
/// kAudioHardwareServiceDeviceProperty_VirtualMainVolume ('vmvc'); the constant
/// lives in a deprecated header that isn't bound.
const VIRTUAL_MAIN_VOLUME: u32 = 0x766d_7663;

// ---- Property addresses ----

pub fn scope(direction: Direction) -> u32 {
    match direction {
        Direction::Output => kAudioObjectPropertyScopeOutput,
        Direction::Input => kAudioObjectPropertyScopeInput,
    }
}

pub fn default_selector(direction: Direction) -> u32 {
    match direction {
        Direction::Output => kAudioHardwarePropertyDefaultOutputDevice,
        Direction::Input => kAudioHardwarePropertyDefaultInputDevice,
    }
}

pub fn scoped(selector: u32, scope: u32, element: u32) -> AudioObjectPropertyAddress {
    AudioObjectPropertyAddress {
        mSelector: selector,
        mScope: scope,
        mElement: element,
    }
}

pub fn global(selector: u32) -> AudioObjectPropertyAddress {
    scoped(
        selector,
        kAudioObjectPropertyScopeGlobal,
        kAudioObjectPropertyElementMain,
    )
}

/// Settable volume properties: the virtual main volume, else channels 1 and 2.
pub fn volume_addrs(device: u32, direction: Direction) -> Vec<AudioObjectPropertyAddress> {
    settable_main_or_channels(
        device,
        VIRTUAL_MAIN_VOLUME,
        kAudioDevicePropertyVolumeScalar,
        direction,
    )
}

pub fn mute_addrs(device: u32, direction: Direction) -> Vec<AudioObjectPropertyAddress> {
    settable_main_or_channels(
        device,
        kAudioDevicePropertyMute,
        kAudioDevicePropertyMute,
        direction,
    )
}

pub fn settable_main_or_channels(
    device: u32,
    main_selector: u32,
    channel_selector: u32,
    direction: Direction,
) -> Vec<AudioObjectPropertyAddress> {
    let main = scoped(
        main_selector,
        scope(direction),
        kAudioObjectPropertyElementMain,
    );
    if settable(device, main) {
        return vec![main];
    }
    (1..=2)
        .map(|channel| scoped(channel_selector, scope(direction), channel))
        .filter(|a| settable(device, *a))
        .collect()
}

// ---- Raw CoreAudio access ----

pub fn device_ids() -> Vec<u32> {
    get_vec(SYSTEM, global(kAudioHardwarePropertyDevices))
}

pub fn get<T>(object: u32, mut address: AudioObjectPropertyAddress) -> Option<T> {
    let mut value = MaybeUninit::<T>::zeroed();
    let mut size = size_of::<T>() as u32;
    let status = unsafe {
        AudioObjectGetPropertyData(
            object,
            NonNull::from(&mut address),
            0,
            null(),
            NonNull::from(&mut size),
            NonNull::from(&mut value).cast(),
        )
    };
    // Only called with plain numbers and pointers, for which zeroed is valid.
    (status == 0).then(|| unsafe { value.assume_init() })
}

pub fn get_vec<T: Default + Clone>(object: u32, mut address: AudioObjectPropertyAddress) -> Vec<T> {
    let mut size = 0u32;
    let status = unsafe {
        AudioObjectGetPropertyDataSize(
            object,
            NonNull::from(&mut address),
            0,
            null(),
            NonNull::from(&mut size),
        )
    };
    if status != 0 || size == 0 {
        return Vec::new();
    }
    let mut items = vec![T::default(); size as usize / size_of::<T>()];
    let status = unsafe {
        AudioObjectGetPropertyData(
            object,
            NonNull::from(&mut address),
            0,
            null(),
            NonNull::from(&mut size),
            NonNull::new_unchecked(items.as_mut_ptr()).cast(),
        )
    };
    if status != 0 {
        return Vec::new();
    }
    items.truncate(size as usize / size_of::<T>());
    items
}

pub fn get_string(object: u32, selector: u32) -> Option<String> {
    let raw: *const CFString = get(object, global(selector))?;
    // CoreAudio hands over a retained string.
    let string = unsafe { CFRetained::from_raw(NonNull::new(raw.cast_mut())?) };
    Some(string.to_string())
}

pub fn set<T>(object: u32, mut address: AudioObjectPropertyAddress, mut value: T) -> bool {
    let status = unsafe {
        AudioObjectSetPropertyData(
            object,
            NonNull::from(&mut address),
            0,
            null(),
            size_of::<T>() as u32,
            NonNull::from(&mut value).cast(),
        )
    };
    status == 0
}

pub fn settable(object: u32, mut address: AudioObjectPropertyAddress) -> bool {
    let mut settable = 0u8;
    let status = unsafe {
        AudioObjectIsPropertySettable(
            object,
            NonNull::from(&mut address),
            NonNull::from(&mut settable),
        )
    };
    status == 0 && settable != 0
}

// ---- Listeners ----

pub type ListenerBlock = RcBlock<dyn Fn(u32, NonNull<AudioObjectPropertyAddress>)>;

/// Listens on the system object plus every device's volume and mute.
pub struct Listeners {
    block: ListenerBlock,
    watched: Vec<u32>,
}

impl Listeners {
    pub fn new(tx: mpsc::Sender<Msg>) -> Listeners {
        let block = RcBlock::new(move |_: u32, _: NonNull<AudioObjectPropertyAddress>| {
            let _ = tx.send(Msg::Changed);
        });
        let listeners = Listeners {
            block,
            watched: Vec::new(),
        };
        let system = [
            kAudioHardwarePropertyDevices,
            kAudioHardwarePropertyDefaultOutputDevice,
            kAudioHardwarePropertyDefaultInputDevice,
        ];
        for selector in system {
            listeners.add(SYSTEM, global(selector));
        }
        listeners
    }

    /// Re-registers the per-device listeners when the device list changed.
    pub fn watch(&mut self, devices: &[u32]) {
        if self.watched == devices {
            return;
        }
        for device in std::mem::take(&mut self.watched) {
            for address in device_addresses() {
                self.remove(device, address);
            }
        }
        for device in devices {
            for address in device_addresses() {
                self.add(*device, address);
            }
        }
        self.watched = devices.to_vec();
    }

    fn add(&self, object: u32, mut address: AudioObjectPropertyAddress) {
        unsafe {
            AudioObjectAddPropertyListenerBlock(
                object,
                NonNull::from(&mut address),
                None,
                RcBlock::as_ptr(&self.block).cast(),
            );
        }
    }

    fn remove(&self, object: u32, mut address: AudioObjectPropertyAddress) {
        unsafe {
            AudioObjectRemovePropertyListenerBlock(
                object,
                NonNull::from(&mut address),
                None,
                RcBlock::as_ptr(&self.block).cast(),
            );
        }
    }
}

pub fn device_addresses() -> Vec<AudioObjectPropertyAddress> {
    let mut addresses = Vec::new();
    for scope in [
        kAudioObjectPropertyScopeOutput,
        kAudioObjectPropertyScopeInput,
    ] {
        addresses.push(scoped(
            VIRTUAL_MAIN_VOLUME,
            scope,
            kAudioObjectPropertyElementMain,
        ));
        for element in 0..=2 {
            addresses.push(scoped(kAudioDevicePropertyVolumeScalar, scope, element));
            addresses.push(scoped(kAudioDevicePropertyMute, scope, element));
        }
    }
    addresses
}

pub fn has_streams(device: u32, direction: Direction) -> bool {
    let mut a = scoped(kAudioDevicePropertyStreams, scope(direction), 0);
    let mut size = 0u32;
    let status = unsafe {
        AudioObjectGetPropertyDataSize(
            device,
            NonNull::from(&mut a),
            0,
            null(),
            NonNull::from(&mut size),
        )
    };
    status == 0 && size > 0
}
