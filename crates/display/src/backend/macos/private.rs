//! Private frameworks, loaded at runtime like `brightness-panel` does:
//! DisplayServices (screen brightness), CoreBrightness (keyboard backlight and
//! Night Shift) and CoreDisplay (display names). They can change between
//! macOS releases, so every lookup may come back empty.

use super::cf::{self, CFRef};
use objc2::{
    msg_send,
    rc::Retained,
    runtime::{AnyClass, AnyObject, Sel},
    sel,
};
use std::ffi::{CStr, c_char, c_int, c_void};

unsafe extern "C" {
    fn dlopen(path: *const c_char, mode: c_int) -> *mut c_void;
    fn dlsym(handle: *mut c_void, name: *const c_char) -> *mut c_void;
    fn objc_msgSend();
}

const RTLD_NOW: c_int = 2;
const FRAMEWORKS: &str = "/System/Library/PrivateFrameworks";

fn load(path: &str) -> Option<*mut c_void> {
    let path = std::ffi::CString::new(path).ok()?;
    // Safety: plain dlopen of a system path.
    let handle = unsafe { dlopen(path.as_ptr(), RTLD_NOW) };
    (!handle.is_null()).then_some(handle)
}

fn symbol(handle: *mut c_void, name: &CStr) -> Option<*mut c_void> {
    // Safety: plain dlsym on a handle from `load`.
    let found = unsafe { dlsym(handle, name.as_ptr()) };
    (!found.is_null()).then_some(found)
}

// ---- DisplayServices ----

type GetBrightness = unsafe extern "C" fn(u32, *mut f32) -> i32;
type SetBrightness = unsafe extern "C" fn(u32, f32) -> i32;

pub struct DisplayServices {
    get: GetBrightness,
    set: SetBrightness,
}

impl DisplayServices {
    pub fn load() -> Option<Self> {
        let handle = load(&format!(
            "{FRAMEWORKS}/DisplayServices.framework/DisplayServices"
        ))?;
        let get = symbol(handle, c"DisplayServicesGetBrightness")?;
        let set = symbol(handle, c"DisplayServicesSetBrightness")?;
        // Safety: these are the documented-by-use signatures of both symbols.
        unsafe {
            Some(Self {
                get: std::mem::transmute::<*mut c_void, GetBrightness>(get),
                set: std::mem::transmute::<*mut c_void, SetBrightness>(set),
            })
        }
    }

    /// None for displays macOS cannot dim itself (most external monitors).
    pub fn get(&self, display: u32) -> Option<f32> {
        let mut value = 0.0f32;
        // Safety: `value` outlives the call.
        let status = unsafe { (self.get)(display, &mut value) };
        (status == 0).then_some(value)
    }

    pub fn set(&self, display: u32, value: f32) -> bool {
        // Safety: plain call.
        unsafe { (self.set)(display, value) == 0 }
    }
}

// ---- Display names ----

/// The product name macOS shows for a display, e.g. "LG UltraFine".
pub fn display_name(display: u32) -> Option<String> {
    type Info = unsafe extern "C" fn(u32) -> CFRef;
    let handle = load("/System/Library/Frameworks/CoreDisplay.framework/CoreDisplay")?;
    let create = symbol(handle, c"CoreDisplay_DisplayCreateInfoDictionary")?;
    // Safety: the symbol takes a display id and returns an owned dictionary.
    let create = unsafe { std::mem::transmute::<*mut c_void, Info>(create) };
    let info = unsafe { create(display) };
    if info.is_null() {
        return None;
    }
    let names = cf::dict_get(info, c"DisplayProductName");
    let name = cf::string_of(cf::dict_get(names, c"en_US"));
    cf::release(info);
    name
}

// ---- Keyboard backlight ----

pub struct Keyboard {
    client: Retained<AnyObject>,
    id: u64,
}

impl Keyboard {
    /// None on Macs without a backlit keyboard.
    pub fn open() -> Option<Self> {
        load(&format!(
            "{FRAMEWORKS}/CoreBrightness.framework/CoreBrightness"
        ))?;
        let class = AnyClass::get(c"KeyboardBrightnessClient")?;
        // Safety: these selectors are the ones `brightness-panel` calls, with
        // the same argument and return types.
        unsafe {
            let client: Retained<AnyObject> = msg_send![class, new];
            let ids: Option<Retained<AnyObject>> = msg_send![&client, copyKeyboardBacklightIDs];
            let ids = ids?;
            let count: usize = msg_send![&ids, count];
            if count == 0 {
                return None;
            }
            let first: Retained<AnyObject> = msg_send![&ids, objectAtIndex: 0usize];
            let id: u64 = msg_send![&first, unsignedLongLongValue];
            Some(Self { client, id })
        }
    }

    pub fn get(&self) -> f32 {
        // Safety: see `open`.
        unsafe { msg_send![&self.client, brightnessForKeyboard: self.id] }
    }

    pub fn set(&self, value: f32) -> bool {
        // Safety: see `open`.
        unsafe { msg_send![&self.client, setBrightness: value, forKeyboard: self.id] }
    }
}

// ---- Night Shift ----

/// `CBBlueLightStatus`, in the layout the private header declares.
#[repr(C)]
#[derive(Default)]
struct BlueLightStatus {
    active: bool,
    enabled: bool,
    sun_schedule_permitted: bool,
    mode: i32,
    schedule: [i32; 4],
    disable_flags: u64,
    available: bool,
}

pub struct BlueLight {
    client: Retained<AnyObject>,
}

type Status = unsafe extern "C" fn(*const AnyObject, Sel, *mut BlueLightStatus) -> bool;
type Strength = unsafe extern "C" fn(*const AnyObject, Sel, *mut f32) -> bool;
type SetStrength = unsafe extern "C" fn(*const AnyObject, Sel, f32, bool) -> bool;
type SetEnabled = unsafe extern "C" fn(*const AnyObject, Sel, bool) -> bool;

impl BlueLight {
    pub fn open() -> Option<Self> {
        load(&format!(
            "{FRAMEWORKS}/CoreBrightness.framework/CoreBrightness"
        ))?;
        let class = AnyClass::get(c"CBBlueLightClient")?;
        // Safety: plain `new` on a class we just found.
        let client: Retained<AnyObject> = unsafe { msg_send![class, new] };
        Some(Self { client })
    }

    /// (on, warmth 0.0-1.0), or None when Night Shift is unsupported here.
    pub fn get(&self) -> Option<(bool, f32)> {
        let mut status = BlueLightStatus::default();
        let mut strength = 0.0f32;
        // Safety: the struct matches the private header and both out
        // pointers outlive the calls. objc_msgSend is cast to each call's
        // real signature, so the runtime does not check the types.
        unsafe {
            let status_fn: Status = std::mem::transmute(objc_msgSend as unsafe extern "C" fn());
            let strength_fn: Strength = std::mem::transmute(objc_msgSend as unsafe extern "C" fn());
            let client = Retained::as_ptr(&self.client);
            if !status_fn(client, sel!(getBlueLightStatus:), &mut status) || !status.available {
                return None;
            }
            strength_fn(client, sel!(getStrength:), &mut strength);
        }
        Some((status.enabled, strength.clamp(0.0, 1.0)))
    }

    pub fn set(&self, on: bool, warmth: f32) -> bool {
        // Safety: see `get`.
        unsafe {
            let strength_fn: SetStrength =
                std::mem::transmute(objc_msgSend as unsafe extern "C" fn());
            let enabled_fn: SetEnabled =
                std::mem::transmute(objc_msgSend as unsafe extern "C" fn());
            let client = Retained::as_ptr(&self.client);
            let a = strength_fn(client, sel!(setStrength:commit:), warmth, true);
            let b = enabled_fn(client, sel!(setEnabled:), on);
            a && b
        }
    }
}
