//! macOS: NSWorkspace for the app list, libproc for memory and CPU, and the
//! SkyLight call Activity Monitor uses for "not responding".

use super::Raw;
use objc2_app_kit::{NSApplicationActivationPolicy, NSRunningApplication, NSWorkspace};
use std::{
    ffi::{c_char, c_void},
    sync::OnceLock,
};

/// The popup's own host app; it must not offer to quit itself.
const HOST_BUNDLE: &str = "io.github.syntheit.telmo";

#[link(name = "CoreFoundation", kind = "framework")]
unsafe extern "C" {
    static kCFRunLoopDefaultMode: *const c_void;
    fn CFRunLoopRunInMode(mode: *const c_void, seconds: f64, return_after_source: bool) -> i32;
}

/// NSWorkspace only learns about launches and quits through the run loop of
/// the thread that first asked it, so let that loop run before each look.
fn pump_run_loop() {
    // SAFETY: plain call with the framework's own default mode constant.
    unsafe { CFRunLoopRunInMode(kCFRunLoopDefaultMode, 0.05, false) };
}

pub fn list() -> Result<Vec<Raw>, String> {
    pump_run_loop();
    let timebase = Timebase::read();
    let detector = detector();
    let mut apps = Vec::new();
    for app in NSWorkspace::sharedWorkspace().runningApplications().iter() {
        if app.activationPolicy() != NSApplicationActivationPolicy::Regular || app.isTerminated() {
            continue;
        }
        let is_host = app
            .bundleIdentifier()
            .is_some_and(|id| id.to_string() == HOST_BUNDLE);
        let Some(name) = app.localizedName().map(|n| n.to_string()) else {
            continue;
        };
        let pid = app.processIdentifier();
        // The app may have exited since the list was made.
        let Some(usage) = usage(pid) else { continue };
        if is_host {
            continue;
        }
        apps.push(Raw {
            pid,
            name,
            memory: usage.ri_phys_footprint,
            cpu_ns: timebase.nanoseconds(usage.ri_user_time + usage.ri_system_time),
            unresponsive: detector.is_some_and(|d| d.is_unresponsive(pid)),
            windows: 0,
        });
    }
    Ok(apps)
}

pub fn quit(pid: i32, name: &str, force: bool) -> Result<(), String> {
    // No app means it already quit.
    let Some(app) = NSRunningApplication::runningApplicationWithProcessIdentifier(pid) else {
        return Ok(());
    };
    let asked = if force {
        app.forceTerminate()
    } else {
        app.terminate()
    };
    if asked {
        return Ok(());
    }
    if force {
        Err(format!(
            "macOS would not force quit {name}. Try Activity Monitor."
        ))
    } else {
        Err(format!(
            "{name} did not accept the request. Press K to force it."
        ))
    }
}

fn usage(pid: i32) -> Option<libc::rusage_info_v2> {
    // SAFETY: an all-zero rusage_info_v2 is valid plain data, and
    // proc_pid_rusage fills at most one rusage_info_v2 through the pointer.
    unsafe {
        let mut info: libc::rusage_info_v2 = std::mem::zeroed();
        let out = (&mut info as *mut libc::rusage_info_v2).cast::<libc::rusage_info_t>();
        (libc::proc_pid_rusage(pid, libc::RUSAGE_INFO_V2, out) == 0).then_some(info)
    }
}

/// Process times are in mach absolute time units, not nanoseconds.
struct Timebase {
    numer: u64,
    denom: u64,
}

impl Timebase {
    // libc marks these deprecated in favor of mach2; one call isn't worth a crate.
    #[allow(deprecated)]
    fn read() -> Self {
        let mut info = libc::mach_timebase_info_data_t { numer: 0, denom: 0 };
        // SAFETY: info is a valid out pointer.
        let ok = unsafe { libc::mach_timebase_info(&mut info) } == 0 && info.denom != 0;
        if ok {
            Self {
                numer: info.numer.into(),
                denom: info.denom.into(),
            }
        } else {
            Self { numer: 1, denom: 1 }
        }
    }

    fn nanoseconds(&self, ticks: u64) -> u64 {
        (u128::from(ticks) * u128::from(self.numer) / u128::from(self.denom)) as u64
    }
}

#[repr(C)]
#[derive(Default)]
struct ProcessSerialNumber {
    high: u32,
    low: u32,
}

/// The private calls behind Activity Monitor's red "Not Responding".
#[derive(Clone, Copy)]
struct Detector {
    connection: i32,
    is_unresponsive: IsUnresponsive,
    process_for_pid: ProcessForPid,
}

type MainConnection = extern "C" fn() -> i32;
type IsUnresponsive = unsafe extern "C" fn(i32, *const ProcessSerialNumber) -> bool;
type ProcessForPid = unsafe extern "C" fn(i32, *mut ProcessSerialNumber) -> i32;

impl Detector {
    fn is_unresponsive(&self, pid: i32) -> bool {
        let mut psn = ProcessSerialNumber::default();
        // SAFETY: both functions were found by name with these signatures;
        // psn is a valid out pointer for the length of the calls.
        unsafe {
            (self.process_for_pid)(pid, &mut psn) == 0
                && (self.is_unresponsive)(self.connection, &psn)
        }
    }
}

/// Loaded once. If any piece is missing (a future macOS), nothing is ever
/// reported as not responding.
fn detector() -> Option<&'static Detector> {
    static DETECTOR: OnceLock<Option<Detector>> = OnceLock::new();
    DETECTOR.get_or_init(load_detector).as_ref()
}

fn load_detector() -> Option<Detector> {
    const SKYLIGHT: &std::ffi::CStr =
        c"/System/Library/PrivateFrameworks/SkyLight.framework/SkyLight";
    const SERVICES: &std::ffi::CStr =
        c"/System/Library/Frameworks/ApplicationServices.framework/ApplicationServices";
    // SAFETY: dlopen/dlsym with valid C strings. The symbols have the
    // signatures in `Detector`, and the libraries stay loaded for good.
    unsafe {
        let skylight = libc::dlopen(SKYLIGHT.as_ptr(), libc::RTLD_LAZY);
        let services = libc::dlopen(SERVICES.as_ptr(), libc::RTLD_LAZY);
        if skylight.is_null() || services.is_null() {
            return None;
        }
        let symbol = |library: *mut c_void, name: &std::ffi::CStr| {
            let found = libc::dlsym(library, name.as_ptr() as *const c_char);
            (!found.is_null()).then_some(found)
        };
        let main_connection = symbol(skylight, c"CGSMainConnectionID")?;
        let is_unresponsive = symbol(skylight, c"CGSEventIsAppUnresponsive")?;
        let process_for_pid = symbol(services, c"GetProcessForPID")?;
        let main_connection = std::mem::transmute::<*mut c_void, MainConnection>(main_connection);
        Some(Detector {
            connection: main_connection(),
            is_unresponsive: std::mem::transmute::<*mut c_void, IsUnresponsive>(is_unresponsive),
            process_for_pid: std::mem::transmute::<*mut c_void, ProcessForPid>(process_for_pid),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_private_calls_load_on_this_macos() {
        assert!(detector().is_some_and(|d| d.connection != 0));
    }
}
