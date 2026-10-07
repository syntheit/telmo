//! The few CoreFoundation and CoreGraphics calls this backend needs, declared
//! by hand. Everything unsafe about displays lives in this file and `ddc.rs`.

use std::ffi::{CStr, c_char, c_void};

pub type CFRef = *const c_void;

const UTF8: u32 = 0x0800_0100;

#[link(name = "CoreFoundation", kind = "framework")]
unsafe extern "C" {
    static kCFTypeDictionaryKeyCallBacks: c_void;
    static kCFTypeDictionaryValueCallBacks: c_void;
    static kCFBooleanTrue: CFRef;
    static kCFRunLoopDefaultMode: CFRef;
    fn CFRelease(cf: CFRef);
    fn CFArrayGetCount(array: CFRef) -> isize;
    fn CFArrayGetValueAtIndex(array: CFRef, index: isize) -> CFRef;
    fn CFDictionaryGetValue(dict: CFRef, key: CFRef) -> CFRef;
    fn CFDictionaryCreate(
        alloc: CFRef,
        keys: *const CFRef,
        values: *const CFRef,
        count: isize,
        key_callbacks: *const c_void,
        value_callbacks: *const c_void,
    ) -> CFRef;
    fn CFStringCreateWithCString(alloc: CFRef, text: *const c_char, encoding: u32) -> CFRef;
    fn CFStringGetCString(s: CFRef, buffer: *mut c_char, size: isize, encoding: u32) -> u8;
    fn CFAbsoluteTimeGetCurrent() -> f64;
    fn CFRunLoopTimerCreate(
        alloc: CFRef,
        fire: f64,
        interval: f64,
        flags: u64,
        order: isize,
        callback: extern "C" fn(CFRef, *mut c_void),
        context: *const c_void,
    ) -> CFRef;
    fn CFRunLoopGetCurrent() -> CFRef;
    fn CFRunLoopAddTimer(run_loop: CFRef, timer: CFRef, mode: CFRef);
    fn CFRunLoopRun();
}

#[link(name = "CoreGraphics", kind = "framework")]
unsafe extern "C" {
    static kCGDisplayShowDuplicateLowResolutionModes: CFRef;
    fn CGGetActiveDisplayList(max: u32, ids: *mut u32, count: *mut u32) -> i32;
    fn CGDisplayIsBuiltin(id: u32) -> u32;
    fn CGDisplayCopyDisplayMode(id: u32) -> CFRef;
    fn CGDisplayCopyAllDisplayModes(id: u32, options: CFRef) -> CFRef;
    fn CGDisplayModeRelease(mode: CFRef);
    fn CGDisplayModeGetWidth(mode: CFRef) -> usize;
    fn CGDisplayModeGetHeight(mode: CFRef) -> usize;
    fn CGDisplayModeGetPixelWidth(mode: CFRef) -> usize;
    fn CGDisplayModeGetPixelHeight(mode: CFRef) -> usize;
    fn CGDisplayModeGetRefreshRate(mode: CFRef) -> f64;
    fn CGDisplayModeGetIOFlags(mode: CFRef) -> u32;
    fn CGDisplayModeIsUsableForDesktopGUI(mode: CFRef) -> bool;
    fn CGBeginDisplayConfiguration(config: *mut CFRef) -> i32;
    fn CGConfigureDisplayWithDisplayMode(
        config: CFRef,
        id: u32,
        mode: CFRef,
        options: CFRef,
    ) -> i32;
    fn CGCompleteDisplayConfiguration(config: CFRef, option: u32) -> i32;
    fn CGCancelDisplayConfiguration(config: CFRef) -> i32;
    fn CGDisplayRegisterReconfigurationCallback(
        callback: extern "C" fn(u32, u32, *mut c_void),
        user: *mut c_void,
    ) -> i32;
}

const CONFIGURE_PERMANENTLY: u32 = 2;
const BEGIN_CONFIGURATION: u32 = 1;
const DEFAULT_MODE_FLAG: u32 = 0x4;

/// One display mode, in points (`width`) and pixels (`pixel_width`).
#[derive(Debug, Clone, Copy)]
pub struct ModeInfo {
    pub width: usize,
    pub height: usize,
    pub pixel_width: usize,
    pub pixel_height: usize,
    pub refresh: f64,
    pub is_default: bool,
    pub usable: bool,
}

impl ModeInfo {
    pub fn hidpi(&self) -> bool {
        self.pixel_width > self.width
    }

    /// Stable across calls; `set_mode` finds the mode again by it.
    pub fn id(&self) -> String {
        format!("{}x{}@{}", self.width, self.height, self.pixel_width)
    }
}

/// Safety: `mode` must be a live CGDisplayModeRef.
unsafe fn read_mode(mode: CFRef) -> ModeInfo {
    unsafe {
        ModeInfo {
            width: CGDisplayModeGetWidth(mode),
            height: CGDisplayModeGetHeight(mode),
            pixel_width: CGDisplayModeGetPixelWidth(mode),
            pixel_height: CGDisplayModeGetPixelHeight(mode),
            refresh: CGDisplayModeGetRefreshRate(mode),
            is_default: CGDisplayModeGetIOFlags(mode) & DEFAULT_MODE_FLAG != 0,
            usable: CGDisplayModeIsUsableForDesktopGUI(mode),
        }
    }
}

pub fn active_displays() -> Vec<u32> {
    let mut ids = [0u32; 16];
    let mut count = 0u32;
    // Safety: the buffer holds the 16 ids we say it does.
    let status = unsafe { CGGetActiveDisplayList(16, ids.as_mut_ptr(), &mut count) };
    if status != 0 {
        return Vec::new();
    }
    ids[..count as usize].to_vec()
}

pub fn is_builtin(id: u32) -> bool {
    // Safety: plain query by id.
    unsafe { CGDisplayIsBuiltin(id) != 0 }
}

pub fn current_mode(id: u32) -> Option<ModeInfo> {
    // Safety: the copy is ours; it is read once and released.
    unsafe {
        let mode = CGDisplayCopyDisplayMode(id);
        if mode.is_null() {
            return None;
        }
        let info = read_mode(mode);
        CGDisplayModeRelease(mode);
        Some(info)
    }
}

/// Runs `f` over every mode the display offers, including the low-resolution
/// duplicates System Settings hides behind HiDPI. `f` may keep the mode ref
/// only for the duration of the call.
fn with_modes<T>(id: u32, f: impl FnOnce(&[(CFRef, ModeInfo)]) -> T) -> Option<T> {
    // Safety: the options dictionary and the mode array are released after
    // `f`; mode refs are borrowed from the array and never escape.
    unsafe {
        let key = kCGDisplayShowDuplicateLowResolutionModes;
        let value = kCFBooleanTrue;
        let options = CFDictionaryCreate(
            std::ptr::null(),
            &key,
            &value,
            1,
            &raw const kCFTypeDictionaryKeyCallBacks,
            &raw const kCFTypeDictionaryValueCallBacks,
        );
        let array = CGDisplayCopyAllDisplayModes(id, options);
        if !options.is_null() {
            CFRelease(options);
        }
        if array.is_null() {
            return None;
        }
        let modes: Vec<_> = (0..CFArrayGetCount(array))
            .map(|i| CFArrayGetValueAtIndex(array, i))
            .map(|mode| (mode, read_mode(mode)))
            .collect();
        let out = f(&modes);
        CFRelease(array);
        Some(out)
    }
}

pub fn all_modes(id: u32) -> Vec<ModeInfo> {
    with_modes(id, |modes| modes.iter().map(|(_, m)| *m).collect()).unwrap_or_default()
}

/// Switch to the mode with this `ModeInfo::id`, permanently.
pub fn set_mode(id: u32, wanted: &str) -> Result<(), String> {
    let refresh = current_mode(id).map_or(0.0, |m| m.refresh);
    let found = with_modes(id, |modes| {
        let matching = modes.iter().filter(|(_, m)| m.id() == wanted);
        let best = matching.min_by(|(_, a), (_, b)| {
            (a.refresh - refresh)
                .abs()
                .total_cmp(&(b.refresh - refresh).abs())
        })?;
        Some(configure(id, best.0))
    });
    match found {
        Some(Some(result)) => result,
        _ => Err("That size is no longer offered. Reopen the app to refresh the list.".into()),
    }
}

fn configure(id: u32, mode: CFRef) -> Result<(), String> {
    let failed = |what: &str| format!("macOS refused to change the size ({what}). Try again.");
    // Safety: a configuration is always either completed or cancelled.
    unsafe {
        let mut config: CFRef = std::ptr::null();
        if CGBeginDisplayConfiguration(&mut config) != 0 {
            return Err(failed("could not start"));
        }
        if CGConfigureDisplayWithDisplayMode(config, id, mode, std::ptr::null()) != 0 {
            CGCancelDisplayConfiguration(config);
            return Err(failed("not accepted"));
        }
        if CGCompleteDisplayConfiguration(config, CONFIGURE_PERMANENTLY) != 0 {
            return Err(failed("could not apply"));
        }
    }
    Ok(())
}

pub fn cf_string(text: &CStr) -> CFRef {
    // Safety: `text` is NUL terminated. The caller owns the result.
    unsafe { CFStringCreateWithCString(std::ptr::null(), text.as_ptr(), UTF8) }
}

pub fn string_of(cf: CFRef) -> Option<String> {
    if cf.is_null() {
        return None;
    }
    let mut buffer = [0 as c_char; 256];
    // Safety: the buffer length is passed along; `cf` is a CFString.
    let ok = unsafe { CFStringGetCString(cf, buffer.as_mut_ptr(), 256, UTF8) };
    // Safety: on success the buffer holds a NUL terminated string.
    (ok != 0).then(|| {
        unsafe { CStr::from_ptr(buffer.as_ptr()) }
            .to_string_lossy()
            .into_owned()
    })
}

pub fn release(cf: CFRef) {
    if !cf.is_null() {
        // Safety: the caller passes a ref it owns.
        unsafe { CFRelease(cf) }
    }
}

/// `dict[key]` for a string key; borrowed, not retained.
pub fn dict_get(dict: CFRef, key: &CStr) -> CFRef {
    let key = cf_string(key);
    // Safety: `dict` is a CFDictionary and `key` a CFString released below.
    let value = unsafe { CFDictionaryGetValue(dict, key) };
    release(key);
    value
}

extern "C" fn no_op(_timer: CFRef, _info: *mut c_void) {}

extern "C" fn reconfigured(_display: u32, flags: u32, user: *mut c_void) {
    // Called before and after each change; only the "after" matters.
    if flags & BEGIN_CONFIGURATION == 0 {
        // Safety: `user` is the leaked `Box<Box<dyn Fn()>>` from `watch_displays`.
        let notify = unsafe { &*(user as *const Box<dyn Fn() + Send>) };
        notify();
    }
}

/// Call `notify` after every display change. Blocks the calling thread in a
/// run loop, which sleeps until the system wakes it.
pub fn watch_displays(notify: Box<dyn Fn() + Send>) {
    let user = Box::into_raw(Box::new(notify)) as *mut c_void;
    // Safety: `user` is never freed, so the callback may use it forever. The
    // far-off timer only keeps the run loop from returning at once.
    unsafe {
        CGDisplayRegisterReconfigurationCallback(reconfigured, user);
        let far = CFAbsoluteTimeGetCurrent() + 1e9;
        let timer = CFRunLoopTimerCreate(std::ptr::null(), far, 0.0, 0, 0, no_op, std::ptr::null());
        CFRunLoopAddTimer(CFRunLoopGetCurrent(), timer, kCFRunLoopDefaultMode);
        CFRunLoopRun();
    }
}
