//! Listens to everything playing on the Mac with a Core Audio process tap
//! (macOS 14.2+): a global stereo tap inside a private aggregate device whose
//! IOProc copies samples into a buffer that this thread analyzes.
//!
//! Without the "System Audio Recording" permission the tap either fails or
//! delivers silence. Both end in `Event::VisualizerBlocked`.

use super::{FRAME, stopped};
use crate::backend::macos::hal::{get, get_string, global};
use crate::backend::{Event, Tx};
use crate::spectrum::Analyzer;
use objc2::{AnyThread, rc::Retained, runtime::AnyObject};
use objc2_core_audio::*;
use objc2_core_audio_types::{AudioBufferList, AudioStreamBasicDescription, AudioTimeStamp};
use objc2_foundation::{NSArray, NSDictionary, NSNumber, NSString};
use std::{
    ffi::c_void,
    ptr::NonNull,
    slice,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU32, Ordering},
    },
    thread,
    time::{Duration, Instant},
};

/// Newest samples to keep between two analysis frames (about 85 ms of stereo).
const KEEP: usize = 8192;
/// Exact silence this long while something plays means we are not allowed in.
const SILENT_FOR: Duration = Duration::from_secs(4);

pub fn run(stop: &AtomicBool, events: &Tx) {
    let samples = Arc::new(Samples::default());
    let Some(tap) = Tap::open(samples.clone()) else {
        let _ = events.send(Event::VisualizerBlocked);
        return;
    };
    let mut analyzer = Analyzer::new(tap.rate);
    let mut heard = Instant::now();
    let mut blocked = false;
    while !stopped(stop) {
        thread::sleep(FRAME);
        let (data, channels) = samples.take();
        if data.iter().any(|s| *s != 0.0) {
            heard = Instant::now();
            blocked = false;
        } else if !blocked && heard.elapsed() > SILENT_FOR {
            blocked = true;
            let _ = events.send(Event::VisualizerBlocked);
        }
        analyzer.push(&data, channels);
        if events.send(Event::Spectrum(analyzer.frame())).is_err() {
            break;
        }
    }
}

/// Filled by the audio thread, drained by ours. The audio thread never waits.
#[derive(Default)]
struct Samples {
    data: Mutex<Vec<f32>>,
    channels: AtomicU32,
}

impl Samples {
    fn add(&self, new: &[f32], channels: u32) {
        let Ok(mut data) = self.data.try_lock() else {
            return;
        };
        self.channels.store(channels, Ordering::Relaxed);
        data.extend_from_slice(new);
        if data.len() > KEEP {
            let extra = data.len() - KEEP;
            data.drain(..extra);
        }
    }

    fn take(&self) -> (Vec<f32>, usize) {
        let data = self.data.lock().map(|mut d| std::mem::take(&mut *d));
        let channels = self.channels.load(Ordering::Relaxed).max(1) as usize;
        (data.unwrap_or_default(), channels)
    }
}

unsafe extern "C-unwind" fn copy_input(
    _device: AudioObjectID,
    _now: NonNull<AudioTimeStamp>,
    input: NonNull<AudioBufferList>,
    _input_time: NonNull<AudioTimeStamp>,
    _output: NonNull<AudioBufferList>,
    _output_time: NonNull<AudioTimeStamp>,
    client: *mut c_void,
) -> i32 {
    // `client` is the `Samples` kept alive by the `Tap` until the IOProc is gone.
    let samples = unsafe { &*(client as *const Samples) };
    let list = unsafe { input.as_ref() };
    let buffer = list.mBuffers[0];
    if list.mNumberBuffers > 0 && !buffer.mData.is_null() {
        let count = buffer.mDataByteSize as usize / size_of::<f32>();
        let data = unsafe { slice::from_raw_parts(buffer.mData as *const f32, count) };
        samples.add(data, buffer.mNumberChannels);
    }
    0
}

/// The tap, the aggregate device around it and the running IOProc. Dropping
/// it undoes whatever was set up, newest first.
struct Tap {
    tap: AudioObjectID,
    aggregate: AudioObjectID,
    io_proc: AudioDeviceIOProcID,
    started: bool,
    rate: f32,
    samples: Arc<Samples>,
}

impl Tap {
    fn open(samples: Arc<Samples>) -> Option<Self> {
        let mut this = Self {
            tap: 0,
            aggregate: 0,
            io_proc: None,
            started: false,
            rate: 48_000.0,
            samples,
        };
        let uid = this.create_tap()?;
        this.create_aggregate(&uid)?;
        this.start()?;
        Some(this)
    }

    /// Creates a global stereo tap of every process but ours, and returns its UID.
    fn create_tap(&mut self) -> Option<String> {
        let ours = own_process_object().into_iter().map(NSNumber::new_u32);
        let excluded = NSArray::from_retained_slice(&ours.collect::<Vec<_>>());
        let description = unsafe {
            let description = CATapDescription::alloc();
            let description =
                CATapDescription::initStereoGlobalTapButExcludeProcesses(description, &excluded);
            description.setName(&NSString::from_str("Telmo visualizer"));
            description.setPrivate(true);
            description.setMuteBehavior(CATapMuteBehavior::Unmuted);
            description
        };
        let mut id = 0;
        let status = unsafe { AudioHardwareCreateProcessTap(Some(&description), &mut id) };
        if status != 0 {
            return None;
        }
        self.tap = id;
        if let Some(format) =
            get::<AudioStreamBasicDescription>(id, global(kAudioTapPropertyFormat))
        {
            self.rate = format.mSampleRate as f32;
        }
        Some(unsafe { description.UUID().UUIDString().to_string() })
    }

    fn create_aggregate(&mut self, tap_uid: &str) -> Option<()> {
        let output = get::<u32>(
            crate::backend::macos::hal::SYSTEM,
            global(kAudioHardwarePropertyDefaultOutputDevice),
        )
        .and_then(|id| get_string(id, kAudioDevicePropertyDeviceUID))?;
        let aggregate_uid = format!("io.telmo.visualizer.{}", std::process::id());

        let tap = dictionary(&[
            ("uid", object(NSString::from_str(tap_uid))),
            ("drift", object(NSNumber::new_i32(1))),
        ]);
        let sub_device = dictionary(&[("uid", object(NSString::from_str(&output)))]);
        let description = dictionary(&[
            ("uid", object(NSString::from_str(&aggregate_uid))),
            ("name", object(NSString::from_str("Telmo visualizer"))),
            ("master", object(NSString::from_str(&output))),
            ("private", object(NSNumber::new_i32(1))),
            ("stacked", object(NSNumber::new_i32(0))),
            ("tapautostart", object(NSNumber::new_i32(1))),
            (
                "subdevices",
                object(NSArray::from_retained_slice(&[sub_device])),
            ),
            ("taplist", object(NSArray::from_retained_slice(&[tap]))),
        ]);

        let mut id = 0;
        // NSDictionary and CFDictionary are the same object.
        let cf = unsafe { &*(Retained::as_ptr(&description) as *const _) };
        let status = unsafe { AudioHardwareCreateAggregateDevice(cf, NonNull::from(&mut id)) };
        if status != 0 {
            return None;
        }
        self.aggregate = id;
        Some(())
    }

    fn start(&mut self) -> Option<()> {
        let mut io_proc: AudioDeviceIOProcID = None;
        let client = Arc::as_ptr(&self.samples) as *mut c_void;
        let status = unsafe {
            AudioDeviceCreateIOProcID(
                self.aggregate,
                Some(copy_input),
                client,
                NonNull::from(&mut io_proc),
            )
        };
        if status != 0 {
            return None;
        }
        self.io_proc = io_proc;
        if unsafe { AudioDeviceStart(self.aggregate, self.io_proc) } != 0 {
            return None;
        }
        self.started = true;
        Some(())
    }
}

impl Drop for Tap {
    fn drop(&mut self) {
        unsafe {
            if self.started {
                AudioDeviceStop(self.aggregate, self.io_proc);
            }
            if self.io_proc.is_some() {
                AudioDeviceDestroyIOProcID(self.aggregate, self.io_proc);
            }
            if self.aggregate != 0 {
                AudioHardwareDestroyAggregateDevice(self.aggregate);
            }
            if self.tap != 0 {
                AudioHardwareDestroyProcessTap(self.tap);
            }
        }
    }
}

/// Core Audio's object for this process, so the tap can leave it out.
fn own_process_object() -> Option<AudioObjectID> {
    let mut pid = std::process::id() as i32;
    let mut address = global(kAudioHardwarePropertyTranslatePIDToProcessObject);
    let mut object: AudioObjectID = 0;
    let mut size = size_of::<AudioObjectID>() as u32;
    let status = unsafe {
        AudioObjectGetPropertyData(
            crate::backend::macos::hal::SYSTEM,
            NonNull::from(&mut address),
            size_of::<i32>() as u32,
            (&raw mut pid).cast(),
            NonNull::from(&mut size),
            NonNull::from(&mut object).cast(),
        )
    };
    (status == 0 && object != 0).then_some(object)
}

fn object<T: objc2::Message>(value: Retained<T>) -> Retained<AnyObject> {
    // Every Objective-C object is an AnyObject.
    unsafe { Retained::cast_unchecked(value) }
}

fn dictionary(
    pairs: &[(&str, Retained<AnyObject>)],
) -> Retained<NSDictionary<NSString, AnyObject>> {
    let keys: Vec<Retained<NSString>> = pairs.iter().map(|(k, _)| NSString::from_str(k)).collect();
    let key_refs: Vec<&NSString> = keys.iter().map(|k| &**k).collect();
    let value_refs: Vec<&AnyObject> = pairs.iter().map(|(_, v)| &**v).collect();
    NSDictionary::from_slices(&key_refs, &value_refs)
}
