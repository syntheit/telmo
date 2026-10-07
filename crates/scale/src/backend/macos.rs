//! Reads the trackpad through Apple's private MultitouchSupport framework.
//!
//! Approach and calibration follow TrackWeight (https://github.com/KrishKrosh/TrackWeight,
//! MIT, Krish Shah), which builds on OpenMultitouchSupport (MIT, Takuto Nakamura):
//! the `pressure` field of the first touch is already in grams, and it is only
//! reported while a finger is in contact.

use super::{Event, Tx};
use crate::model::Snapshot;
use std::{
    collections::VecDeque,
    ffi::{CStr, c_char, c_int, c_void},
    sync::{
        Mutex,
        mpsc::{Receiver, RecvTimeoutError, Sender, channel},
    },
    time::{Duration, Instant},
};
use tokio::task::JoinHandle;

const FRAMEWORK: &CStr =
    c"/System/Library/PrivateFrameworks/MultitouchSupport.framework/MultitouchSupport";
const NO_TRACKPAD: &str =
    "No Force Touch trackpad found. Telmo Scale needs a MacBook or a Magic Trackpad.";
const START_FAILED: &str =
    "Could not start the trackpad. Quit other apps that read it and try again.";

/// Frames averaged to smooth out sensor noise.
const AVERAGE: usize = 5;
/// The reading counts as stable once it stayed within this many grams...
const STABLE_SPREAD: f32 = 0.5;
/// ...for this long.
const STABLE_FOR: Duration = Duration::from_secs(1);
/// At most one snapshot per this interval (20 per second).
const SEND_EVERY: Duration = Duration::from_millis(50);

/// Layout of `MTTouch` as used by OpenMultitouchSupport.
#[repr(C)]
#[derive(Clone, Copy)]
struct MtTouch {
    frame: c_int,
    timestamp: f64,
    identifier: c_int,
    state: c_int,
    finger_id: c_int,
    hand_id: c_int,
    normalized: [f32; 4],
    total: f32,
    pressure: f32,
    angle: f32,
    major_axis: f32,
    minor_axis: f32,
    absolute: [f32; 4],
    field14: c_int,
    field15: c_int,
    density: f32,
}

type Device = *mut c_void;
type FrameCallback = extern "C" fn(Device, *const MtTouch, c_int, f64, c_int);

struct Api {
    create_default: unsafe extern "C" fn() -> Device,
    register: unsafe extern "C" fn(Device, FrameCallback),
    unregister: unsafe extern "C" fn(Device, FrameCallback),
    start: unsafe extern "C" fn(Device, c_int) -> c_int,
    stop: unsafe extern "C" fn(Device) -> c_int,
    release: unsafe extern "C" fn(Device),
}

/// One raw frame: the pressure of the first touch, or None when nothing touches.
type Frames = Sender<Option<f32>>;

/// The framework calls a plain C function, so the channel lives in a static.
static FRAMES: Mutex<Option<Frames>> = Mutex::new(None);

extern "C" fn on_frame(_: Device, touches: *const MtTouch, count: c_int, _: f64, _: c_int) {
    let pressure = if count > 0 && !touches.is_null() {
        // SAFETY: the framework passes `count` valid touches.
        Some(unsafe { (*touches).pressure })
    } else {
        None
    };
    if let Ok(frames) = FRAMES.lock()
        && let Some(frames) = frames.as_ref()
    {
        let _ = frames.send(pressure);
    }
}

fn load() -> Result<Api, String> {
    // SAFETY: dlopen/dlsym with valid C strings; each symbol is cast to the
    // signature declared in OpenMultitouchSupport's headers.
    unsafe {
        let lib = libc::dlopen(FRAMEWORK.as_ptr(), libc::RTLD_LAZY);
        if lib.is_null() {
            return Err(NO_TRACKPAD.into());
        }
        // Each symbol is cast to the function type of the field it fills.
        let sym = |name: &CStr| -> Result<*mut c_void, String> {
            let ptr = libc::dlsym(lib, name.as_ptr() as *const c_char);
            if ptr.is_null() {
                Err("This macOS version does not allow reading the trackpad.".into())
            } else {
                Ok(ptr)
            }
        };
        fn cast<T: Copy>(ptr: *mut c_void) -> T {
            // SAFETY: T is always an `extern "C"` fn pointer, the size of `ptr`.
            unsafe { std::mem::transmute_copy(&ptr) }
        }
        Ok(Api {
            create_default: cast(sym(c"MTDeviceCreateDefault")?),
            register: cast(sym(c"MTRegisterContactFrameCallback")?),
            unregister: cast(sym(c"MTUnregisterContactFrameCallback")?),
            start: cast(sym(c"MTDeviceStart")?),
            stop: cast(sym(c"MTDeviceStop")?),
            release: cast(sym(c"MTDeviceRelease")?),
        })
    }
}

/// A running trackpad device; stops and releases it when dropped.
struct Trackpad {
    api: Api,
    device: Device,
}

impl Trackpad {
    fn start(frames: Frames) -> Result<Self, String> {
        let api = load()?;
        // SAFETY: calls into the framework as declared above.
        unsafe {
            let device = (api.create_default)();
            if device.is_null() {
                return Err(NO_TRACKPAD.into());
            }
            if let Ok(mut slot) = FRAMES.lock() {
                *slot = Some(frames);
            }
            (api.register)(device, on_frame);
            if (api.start)(device, 0) != 0 {
                (api.unregister)(device, on_frame);
                (api.release)(device);
                return Err(START_FAILED.into());
            }
            Ok(Self { api, device })
        }
    }
}

impl Drop for Trackpad {
    fn drop(&mut self) {
        // SAFETY: `device` is the one we started.
        unsafe {
            (self.api.unregister)(self.device, on_frame);
            (self.api.stop)(self.device);
            (self.api.release)(self.device);
        }
        if let Ok(mut slot) = FRAMES.lock() {
            *slot = None;
        }
    }
}

/// Moving average plus a stability check over the last second.
struct Filter {
    recent: VecDeque<f32>,
    /// Smoothed values with their time, newest last.
    history: VecDeque<(Instant, f32)>,
}

impl Filter {
    fn new() -> Self {
        Self {
            recent: VecDeque::new(),
            history: VecDeque::new(),
        }
    }

    fn push(&mut self, now: Instant, pressure: Option<f32>) -> Snapshot {
        let Some(pressure) = pressure else {
            self.recent.clear();
            self.history.clear();
            return Snapshot::default();
        };
        self.recent.push_back(pressure);
        if self.recent.len() > AVERAGE {
            self.recent.pop_front();
        }
        let average = self.recent.iter().sum::<f32>() / self.recent.len() as f32;
        self.history.push_back((now, average));
        while self
            .history
            .front()
            .is_some_and(|(t, _)| now.duration_since(*t) > STABLE_FOR + SEND_EVERY * 4)
        {
            self.history.pop_front();
        }
        Snapshot {
            unavailable: None,
            touching: true,
            grams: (average.max(0.0) * 10.0).round() / 10.0,
            stable: self.stable(now),
        }
    }

    fn stable(&self, now: Instant) -> bool {
        let covers = self
            .history
            .front()
            .is_some_and(|(t, _)| now.duration_since(*t) >= STABLE_FOR);
        let window = self
            .history
            .iter()
            .filter(|(t, _)| now.duration_since(*t) <= STABLE_FOR);
        let (min, max) = window.fold((f32::MAX, f32::MIN), |(lo, hi), (_, v)| {
            (lo.min(*v), hi.max(*v))
        });
        covers && max - min <= STABLE_SPREAD
    }
}

pub fn spawn(events: Tx) -> JoinHandle<()> {
    tokio::task::spawn_blocking(move || run(events))
}

fn run(events: Tx) {
    let (frames, rx) = channel();
    let _trackpad = match Trackpad::start(frames) {
        Ok(trackpad) => trackpad,
        Err(reason) => {
            let snapshot = Snapshot {
                unavailable: Some(reason),
                ..Snapshot::default()
            };
            let _ = events.send(Event::Snapshot(snapshot));
            return;
        }
    };
    if events.send(Event::Snapshot(Snapshot::default())).is_err() {
        return;
    }
    forward(&rx, &events);
}

/// Turn frames into snapshots: only when something changed, at most every
/// `SEND_EVERY`. While nothing touches the trackpad this just sleeps.
fn forward(rx: &Receiver<Option<f32>>, events: &Tx) {
    let mut filter = Filter::new();
    let mut sent = Snapshot::default();
    let mut latest = sent.clone();
    let mut last_send = Instant::now();
    loop {
        match rx.recv_timeout(SEND_EVERY) {
            Ok(pressure) => latest = filter.push(Instant::now(), pressure),
            Err(RecvTimeoutError::Timeout) => {
                if events.is_closed() {
                    return;
                }
                // Frames stopped; the stability window may still have run out.
                if latest.touching && !latest.stable {
                    latest.stable = filter.stable(Instant::now());
                }
            }
            Err(RecvTimeoutError::Disconnected) => return,
        }
        if latest != sent && last_send.elapsed() >= SEND_EVERY {
            if events.send(Event::Snapshot(latest.clone())).is_err() {
                return;
            }
            sent = latest.clone();
            last_send = Instant::now();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(start: Instant, ms: u64) -> Instant {
        start + Duration::from_millis(ms)
    }

    #[test]
    fn averages_and_rounds() {
        let t0 = Instant::now();
        let mut filter = Filter::new();
        filter.push(t0, Some(100.0));
        let snapshot = filter.push(at(t0, 10), Some(101.0));
        assert_eq!(snapshot.grams, 100.5);
        assert!(snapshot.touching);
    }

    #[test]
    fn lift_resets() {
        let t0 = Instant::now();
        let mut filter = Filter::new();
        filter.push(t0, Some(100.0));
        assert_eq!(filter.push(at(t0, 10), None), Snapshot::default());
    }

    #[test]
    fn stable_after_a_steady_second() {
        let t0 = Instant::now();
        let mut filter = Filter::new();
        let mut last = Snapshot::default();
        for i in 0..=120 {
            last = filter.push(at(t0, i * 10), Some(124.5));
            assert_eq!(last.stable, i >= 100, "frame {i}");
        }
        assert!(last.stable);
    }

    #[test]
    fn moving_value_is_not_stable() {
        let t0 = Instant::now();
        let mut filter = Filter::new();
        for i in 0..=150 {
            let snapshot = filter.push(at(t0, i * 10), Some(i as f32));
            assert!(!snapshot.stable);
        }
    }

    /// Run by hand on a Mac: starts the device for 2 s and prints what arrives.
    #[test]
    #[ignore]
    fn device_starts() {
        let (frames, rx) = channel();
        let trackpad = Trackpad::start(frames).expect("device starts");
        let end = Instant::now() + Duration::from_secs(2);
        let (mut count, mut last) = (0, None);
        while let Some(left) = end.checked_duration_since(Instant::now()) {
            if let Ok(pressure) = rx.recv_timeout(left) {
                count += 1;
                last = pressure;
            }
        }
        drop(trackpad);
        println!("frames: {count}, last raw pressure: {last:?}");
    }
}
