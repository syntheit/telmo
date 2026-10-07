//! IOBluetooth backend. IOBluetooth objects are not thread-safe and its
//! notifications and delegates need a run loop, so everything happens on one
//! dedicated thread that sleeps in the run loop until a callback, a command
//! (forwarded by a feeder thread that wakes the loop) or the next deadline.
//! Callbacks only queue a `Note` (see `handler`).

mod battery;
mod classify;
mod handler;

use super::{Cmd, Event, Rx, Tx};
use crate::model::{Adapter, Device, PairPrompt, Snapshot};
use battery::{BudsCache, HidBattery};
use block2::RcBlock;
use handler::Handler;
use objc2::{
    msg_send, rc::Retained, rc::autoreleasepool, runtime::AnyObject, runtime::NSObjectProtocol, sel,
};
use objc2_core_foundation::{
    CFAbsoluteTimeGetCurrent, CFRetained, CFRunLoop, CFRunLoopTimer, CFType, kCFRunLoopDefaultMode,
};
use objc2_foundation::{NSArray, NSString};
use objc2_io_bluetooth::{
    BluetoothHCIPowerState, BluetoothPINCode, IOBluetoothDevice, IOBluetoothDeviceInquiry,
    IOBluetoothDevicePair, IOBluetoothHostController, IOBluetoothUserNotification,
};
use std::{
    collections::{HashMap, HashSet},
    ffi::{c_int, c_ulong, c_void},
    sync::{
        Arc,
        atomic::Ordering,
        mpsc::{Receiver, Sender, TryRecvError, channel},
    },
    thread,
    time::{Duration, Instant},
};

/// How often to look at connecting devices, which have no completion we can wait on.
const CONNECT_POLL: Duration = Duration::from_millis(200);
const IDLE: Duration = Duration::from_secs(3600);
const DEBOUNCE: Duration = Duration::from_millis(30);
const POWER_POLL: Duration = Duration::from_secs(2);
const POWER_POLL_WHILE_SWITCHING: Duration = Duration::from_millis(100);
const POWER_SWITCH_TIMEOUT: Duration = Duration::from_secs(8);
const CONNECT_TIMEOUT: Duration = Duration::from_secs(15);
const PAIR_TIMEOUT: Duration = Duration::from_secs(90);
const DISCONNECT_RETRY: Duration = Duration::from_millis(300);
const DISCONNECT_ATTEMPTS: u32 = 5;
const INQUIRY_SECONDS: u8 = 10;
/// Page timeout in units of 0.625 ms: 10 seconds.
const PAGE_TIMEOUT: u16 = 16_000;

const NO_POWER_API: &str = "Can't change Bluetooth power here. Use System Settings.";
const IOBLUETOOTH: &std::ffi::CStr =
    c"/System/Library/Frameworks/IOBluetooth.framework/IOBluetooth";

pub enum Note {
    Changed,
    Connected(String),
    Disconnected,
    ConnectionComplete(String, i32),
    InquiryComplete,
    Prompt(PairPrompt),
    PairingFinished(i32),
}

pub fn spawn(cmds: Rx, events: Tx) {
    let thread_events = events.clone();
    let spawned = thread::Builder::new()
        .name("bt".into())
        .spawn(move || run(cmds, thread_events));
    if spawned.is_err() {
        let _ = events.send(Event::Snapshot(Snapshot::default()));
    }
}

/// Wakes the run-loop thread from any thread. CFRunLoopPerformBlock and
/// CFRunLoopWakeUp are thread-safe.
pub struct Waker(CFRetained<CFRunLoop>);
unsafe impl Send for Waker {}
unsafe impl Sync for Waker {}

impl Waker {
    fn current() -> Option<Waker> {
        CFRunLoop::current().map(Waker)
    }

    pub fn wake(&self) {
        // The queued block stops the loop, so `run_in_mode` returns even if it
        // wasn't running yet.
        let target = self.0.clone();
        let block = RcBlock::new(move || target.stop());
        let mode: Option<&CFType> = unsafe { kCFRunLoopDefaultMode }.map(|mode| mode.as_ref());
        unsafe { self.0.perform_block(mode, Some(&block)) };
        self.0.wake_up();
    }
}

fn run(cmds: Rx, events: Tx) {
    let Some(waker) = Waker::current() else {
        let _ = events.send(Event::Snapshot(Snapshot::default()));
        return;
    };
    let waker = Arc::new(waker);
    let _keepalive = keep_loop_running();
    let commands = forward_commands(cmds, waker.clone());
    let mut backend = Backend::new(commands, events, waker);
    backend.start();
    while autoreleasepool(|_| {
        if !backend.tick() {
            return false;
        }
        let mode = unsafe { kCFRunLoopDefaultMode };
        CFRunLoop::run_in_mode(mode, backend.sleep().as_secs_f64(), true);
        true
    }) {}
}

/// A run loop with no sources returns at once; a timer that never fires in
/// practice keeps it sleeping until woken.
fn keep_loop_running() -> Option<CFRetained<CFRunLoopTimer>> {
    let block = RcBlock::new(|_: *mut CFRunLoopTimer| {});
    let far_future = CFAbsoluteTimeGetCurrent() + 1e9;
    let timer = unsafe { CFRunLoopTimer::with_handler(None, far_future, 0.0, 0, 0, Some(&block)) }?;
    CFRunLoop::current()?.add_timer(Some(&timer), unsafe { kCFRunLoopDefaultMode });
    Some(timer)
}

/// Hands each command to the run-loop thread; the channel closing ends it.
fn forward_commands(mut cmds: Rx, waker: Arc<Waker>) -> Receiver<Cmd> {
    let (sender, receiver) = channel();
    let _ = thread::Builder::new()
        .name("bt-commands".into())
        .spawn(move || {
            while let Some(cmd) = cmds.blocking_recv() {
                if sender.send(cmd).is_err() {
                    return;
                }
                waker.wake();
            }
            drop(sender);
            waker.wake();
        });
    receiver
}

/// The private power functions, loaded at runtime.
struct Power {
    get: Option<extern "C" fn() -> c_int>,
    set: Option<extern "C" fn(c_int)>,
}

impl Power {
    fn load() -> Self {
        let handle = unsafe { libc::dlopen(IOBLUETOOTH.as_ptr(), libc::RTLD_LAZY) };
        if handle.is_null() {
            return Self {
                get: None,
                set: None,
            };
        }
        let symbol = |name: &std::ffi::CStr| {
            let pointer = unsafe { libc::dlsym(handle, name.as_ptr()) };
            (!pointer.is_null()).then_some(pointer)
        };
        unsafe {
            Self {
                get: symbol(c"IOBluetoothPreferenceGetControllerPowerState")
                    .map(|p| std::mem::transmute::<*mut c_void, extern "C" fn() -> c_int>(p)),
                set: symbol(c"IOBluetoothPreferenceSetControllerPowerState")
                    .map(|p| std::mem::transmute::<*mut c_void, extern "C" fn(c_int)>(p)),
            }
        }
    }

    fn is_on(&self) -> bool {
        match self.get {
            Some(get) => get() != 0,
            None => unsafe {
                IOBluetoothHostController::defaultController()
                    .is_some_and(|c| c.powerState() == BluetoothHCIPowerState::ON)
            },
        }
    }
}

struct PowerSwitch {
    on: bool,
    deadline: Instant,
}

struct Connecting {
    name: String,
    deadline: Instant,
}

struct Disconnecting {
    id: String,
    name: String,
    attempts: u32,
    next_try: Instant,
}

struct Pairing {
    id: String,
    name: String,
    pair: Retained<IOBluetoothDevicePair>,
    prompt: Option<PairPrompt>,
    deadline: Instant,
}

struct Backend {
    cmds: Receiver<Cmd>,
    events: Tx,
    notes: Receiver<Note>,
    power: Power,
    powered: bool,
    next_power_poll: Instant,
    power_switch: Option<PowerSwitch>,
    inquiry: Option<Retained<IOBluetoothDeviceInquiry>>,
    scanning: bool,
    connect_notification: Option<Retained<IOBluetoothUserNotification>>,
    disconnect_notifications: HashMap<String, Retained<IOBluetoothUserNotification>>,
    connecting: HashMap<String, Connecting>,
    disconnecting: Vec<Disconnecting>,
    pairing: Option<Pairing>,
    dirty_since: Option<Instant>,
    buds: Arc<BudsCache>,
    refresh_buds: Sender<()>,
    // Last, so it outlives everything that uses it as a delegate or target.
    handler: Retained<Handler>,
}

impl Backend {
    fn new(cmds: Receiver<Cmd>, events: Tx, waker: Arc<Waker>) -> Self {
        let (note_tx, notes) = channel();
        let buds = Arc::new(BudsCache::default());
        let refresh_buds = battery::spawn_profiler(buds.clone(), note_tx.clone(), waker);
        Self {
            cmds,
            events,
            notes,
            power: Power::load(),
            powered: false,
            next_power_poll: Instant::now(),
            power_switch: None,
            inquiry: None,
            scanning: false,
            connect_notification: None,
            disconnect_notifications: HashMap::new(),
            connecting: HashMap::new(),
            disconnecting: Vec::new(),
            pairing: None,
            dirty_since: None,
            buds,
            refresh_buds,
            handler: Handler::new(note_tx),
        }
    }

    fn start(&mut self) {
        self.powered = self.power.is_on();
        self.connect_notification = unsafe {
            IOBluetoothDevice::registerForConnectNotifications_selector(
                Some(self.delegate()),
                Some(sel!(connected:device:)),
            )
        };
        self.publish();
    }

    fn delegate(&self) -> &AnyObject {
        self.handler.as_ref()
    }

    /// How long the run loop may sleep: until the earliest thing that needs a look.
    fn sleep(&self) -> Duration {
        let mut deadlines = vec![self.next_power_poll];
        deadlines.extend(self.dirty_since.map(|since| since + DEBOUNCE));
        deadlines.extend(self.disconnecting.iter().map(|job| job.next_try));
        deadlines.extend(self.pairing.iter().map(|p| p.deadline));
        let earliest = deadlines.into_iter().min().unwrap_or_else(Instant::now);
        let mut sleep = earliest.saturating_duration_since(Instant::now());
        if !self.connecting.is_empty() {
            sleep = sleep.min(CONNECT_POLL);
        }
        sleep.min(IDLE)
    }

    /// Returns false once the UI is gone and the thread should end.
    fn tick(&mut self) -> bool {
        loop {
            match self.cmds.try_recv() {
                Ok(cmd) => self.handle(cmd),
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => return false,
            }
        }
        while let Ok(note) = self.notes.try_recv() {
            self.note(note);
        }
        self.poll_power();
        self.poll_connecting();
        self.poll_disconnecting();
        self.poll_pairing();
        if self
            .dirty_since
            .is_some_and(|since| since.elapsed() >= DEBOUNCE)
        {
            self.publish();
        }
        true
    }

    fn mark_dirty(&mut self) {
        self.dirty_since.get_or_insert_with(Instant::now);
    }

    fn send(&self, event: Event) {
        let _ = self.events.send(event);
    }

    /// Ends an action: the snapshot goes out first so the row is already right.
    fn done(&mut self, target: &str, result: Result<String, String>) {
        if self.dirty_since.is_some() {
            self.publish();
        }
        self.send(Event::Done {
            target: target.to_string(),
            result,
        });
    }

    fn publish(&mut self) {
        self.dirty_since = None;
        let snapshot = self.snapshot();
        let connected: HashSet<&str> = snapshot
            .devices
            .iter()
            .filter(|d| d.connected)
            .map(|d| d.id.as_str())
            .collect();
        let any_connected = !connected.is_empty();
        if any_connected && !self.buds.any_connected.swap(true, Ordering::Relaxed) {
            let _ = self.refresh_buds.send(());
        }
        self.buds
            .any_connected
            .store(any_connected, Ordering::Relaxed);
        self.watch_disconnects(&connected);
        self.send(Event::Snapshot(snapshot));
    }

    /// A disconnect notification fires once, so re-register after each connect.
    fn watch_disconnects(&mut self, connected: &HashSet<&str>) {
        self.disconnect_notifications.retain(|id, notification| {
            let keep = connected.contains(id.as_str());
            if !keep {
                unsafe { notification.unregister() };
            }
            keep
        });
        for id in connected {
            if self.disconnect_notifications.contains_key(*id) {
                continue;
            }
            let Some(device) = find_device(id) else {
                continue;
            };
            let notification = unsafe {
                device.registerForDisconnectNotification_selector(
                    Some(self.delegate()),
                    Some(sel!(disconnected:device:)),
                )
            };
            if let Some(notification) = notification {
                self.disconnect_notifications
                    .insert((*id).to_string(), notification);
            }
        }
    }

    // Snapshot

    fn snapshot(&self) -> Snapshot {
        Snapshot {
            adapter: Some(Adapter {
                name: controller_name(),
                powered: self.powered,
                discovering: self.powered && self.scanning,
            }),
            devices: self.devices(),
        }
    }

    fn devices(&self) -> Vec<Device> {
        let hid = battery::hid_batteries();
        let mut seen = HashSet::new();
        let mut devices = Vec::new();
        let paired = devices_in(unsafe { IOBluetoothDevice::pairedDevices() });
        let found = self
            .inquiry
            .iter()
            .flat_map(|inquiry| devices_in(unsafe { inquiry.foundDevices() }));
        for device in paired.into_iter().chain(found) {
            if let Some(device) = self.describe(&device, &hid)
                && seen.insert(device.id.clone())
            {
                devices.push(device);
            }
        }
        devices.sort_by(|a, b| {
            let key = |d: &Device| (!d.paired, d.name.to_lowercase());
            key(a).cmp(&key(b))
        });
        devices
    }

    fn describe(&self, device: &IOBluetoothDevice, hid: &[HidBattery]) -> Option<Device> {
        let id = address_of(device)?;
        let name = device_name(device);
        let paired = unsafe { device.isPaired() };
        if paired && name.is_empty() {
            return None;
        }
        let connected = self.powered && unsafe { device.isConnected() };
        let battery = connected
            .then(|| {
                self.buds
                    .get(&id)
                    .or_else(|| battery::single_battery(hid, &id, &name))
            })
            .flatten();
        Some(Device {
            kind: classify::kind(unsafe { device.classOfDevice() }),
            rssi: if paired { None } else { rssi_of(device) },
            id,
            name,
            paired,
            connected,
            auto_connect: None,
            battery,
        })
    }

    // Notes from callbacks

    fn note(&mut self, note: Note) {
        match note {
            Note::Changed => self.mark_dirty(),
            Note::Connected(id) => {
                // Drop the spent disconnect notification; publish re-registers.
                if let Some(old) = self.disconnect_notifications.remove(&id) {
                    unsafe { old.unregister() };
                }
                self.mark_dirty();
            }
            Note::Disconnected => self.mark_dirty(),
            Note::ConnectionComplete(id, status) => self.connection_complete(&id, status),
            Note::InquiryComplete => self.inquiry_complete(),
            Note::Prompt(prompt) => self.pairing_prompt(prompt),
            Note::PairingFinished(status) => self.pairing_finished(status),
        }
    }

    // Power

    fn poll_power(&mut self) {
        if Instant::now() < self.next_power_poll {
            return;
        }
        let interval = if self.power_switch.is_some() {
            POWER_POLL_WHILE_SWITCHING
        } else {
            POWER_POLL
        };
        self.next_power_poll = Instant::now() + interval;
        let on = self.power.is_on();
        if on != self.powered {
            self.powered = on;
            if !on {
                self.stop_scan();
            }
            self.mark_dirty();
        }
        self.finish_power_switch(on);
    }

    fn finish_power_switch(&mut self, on: bool) {
        let Some(switch) = &self.power_switch else {
            return;
        };
        if switch.on == on {
            let message = if on {
                "Bluetooth is on"
            } else {
                "Bluetooth is off"
            };
            self.power_switch = None;
            self.done("adapter", Ok(message.into()));
        } else if Instant::now() >= switch.deadline {
            let word = if switch.on { "on" } else { "off" };
            let message =
                format!("Bluetooth didn't turn {word}. Try again, or use System Settings.");
            self.power_switch = None;
            self.done("adapter", Err(message));
        }
    }

    fn set_power(&mut self, on: bool) {
        let Some(set) = self.power.set else {
            return self.done("adapter", Err(NO_POWER_API.into()));
        };
        if self.powered == on {
            let message = if on {
                "Bluetooth is on"
            } else {
                "Bluetooth is off"
            };
            return self.done("adapter", Ok(message.into()));
        }
        set(c_int::from(on));
        self.power_switch = Some(PowerSwitch {
            on,
            deadline: Instant::now() + POWER_SWITCH_TIMEOUT,
        });
        self.next_power_poll = Instant::now();
    }

    // Commands

    fn handle(&mut self, cmd: Cmd) {
        match cmd {
            Cmd::SetPower(on) => self.set_power(on),
            Cmd::Connect(id) => self.connect(&id),
            Cmd::Disconnect(id) => self.disconnect(&id),
            Cmd::StartScan => self.start_scan(),
            Cmd::StopScan => self.stop_scan(),
            Cmd::Pair(id) => self.pair(&id),
            Cmd::PairReply(reply) => self.pair_reply(reply),
            Cmd::SetAutoConnect(id, _) => self.done(
                &id,
                Err("macOS reconnects paired devices automatically.".into()),
            ),
            Cmd::Forget(id) => self.forget(&id),
            Cmd::Rename(id, _) => self.done(&id, Err("Renaming isn't supported on macOS.".into())),
        }
    }

    /// Looks the device up, or ends the action with a sentence.
    fn require_device(&mut self, id: &str) -> Option<(Retained<IOBluetoothDevice>, String)> {
        if !self.powered {
            self.done(id, Err("Bluetooth is off. Turn it on first.".into()));
            return None;
        }
        let Some(device) = find_device(id) else {
            self.done(id, Err("That device is no longer available.".into()));
            return None;
        };
        let name = display_name(&device, id);
        Some((device, name))
    }

    fn connect(&mut self, id: &str) {
        let Some((device, name)) = self.require_device(id) else {
            return;
        };
        if unsafe { device.isConnected() } {
            return self.done(id, Ok(format!("Connected to {name}")));
        }
        let status = unsafe {
            device.openConnection_withPageTimeout_authenticationRequired(
                Some(self.delegate()),
                PAGE_TIMEOUT,
                false,
            )
        };
        if status != 0 {
            return self.done(id, Err(unreachable_message(&name)));
        }
        self.connecting.insert(
            id.to_string(),
            Connecting {
                name,
                deadline: Instant::now() + CONNECT_TIMEOUT,
            },
        );
    }

    fn connection_complete(&mut self, id: &str, status: i32) {
        let Some(connecting) = self.connecting.remove(id) else {
            return self.mark_dirty();
        };
        self.mark_dirty();
        let connected = find_device(id).is_some_and(|d| unsafe { d.isConnected() });
        if status == 0 || connected {
            self.done(id, Ok(format!("Connected to {}", connecting.name)));
        } else {
            self.done(id, Err(unreachable_message(&connecting.name)));
        }
    }

    /// Backstop in case the completion callback never arrives.
    fn poll_connecting(&mut self) {
        let now = Instant::now();
        let finished: Vec<String> = self
            .connecting
            .iter()
            .filter(|(id, c)| {
                now >= c.deadline || find_device(id).is_some_and(|d| unsafe { d.isConnected() })
            })
            .map(|(id, _)| id.clone())
            .collect();
        for id in finished {
            self.connection_complete(&id, -1);
        }
    }

    fn disconnect(&mut self, id: &str) {
        let Some((_, name)) = self.require_device(id) else {
            return;
        };
        self.disconnecting.push(Disconnecting {
            id: id.to_string(),
            name,
            attempts: 0,
            next_try: Instant::now(),
        });
    }

    /// closeConnection is unreliable, so retry a few times without sleeping.
    fn poll_disconnecting(&mut self) {
        let now = Instant::now();
        let mut jobs = std::mem::take(&mut self.disconnecting);
        jobs.retain_mut(|job| {
            if now < job.next_try {
                return true;
            }
            let device = find_device(&job.id);
            if !device.as_ref().is_some_and(|d| unsafe { d.isConnected() }) {
                let message = format!("Disconnected {}", job.name);
                self.mark_dirty();
                self.done(&job.id, Ok(message));
                return false;
            }
            if job.attempts >= DISCONNECT_ATTEMPTS {
                let message = format!("{} wouldn't disconnect. Try again.", job.name);
                self.done(&job.id, Err(message));
                return false;
            }
            if let Some(device) = device {
                unsafe { device.closeConnection() };
            }
            job.attempts += 1;
            job.next_try = now + DISCONNECT_RETRY;
            true
        });
        jobs.append(&mut self.disconnecting);
        self.disconnecting = jobs;
    }

    fn forget(&mut self, id: &str) {
        let Some(device) = find_device(id) else {
            return self.done(id, Err("That device is no longer available.".into()));
        };
        let name = display_name(&device, id);
        let remove = sel!(remove);
        if !device.respondsToSelector(remove) {
            return self.done(id, Err(FORGET_IN_SETTINGS.into()));
        }
        // Private API that blueutil also uses for --unpair.
        let _: () = unsafe { msg_send![&*device, remove] };
        self.mark_dirty();
        if unsafe { device.isPaired() } {
            return self.done(id, Err(FORGET_IN_SETTINGS.into()));
        }
        self.done(id, Ok(format!("Forgot {name}")));
    }

    // Scanning

    fn start_scan(&mut self) {
        if !self.powered {
            return self.done(
                "adapter",
                Err("Turn Bluetooth on to scan for devices.".into()),
            );
        }
        let Some(inquiry) = self.inquiry() else {
            return self.done("adapter", Err(SCAN_FAILED.into()));
        };
        unsafe { inquiry.clearFoundDevices() };
        self.scanning = true;
        if !self.run_inquiry() {
            self.scanning = false;
            self.done("adapter", Err(SCAN_FAILED.into()));
        }
        self.mark_dirty();
    }

    fn inquiry(&mut self) -> Option<Retained<IOBluetoothDeviceInquiry>> {
        if self.inquiry.is_none() {
            let inquiry =
                unsafe { IOBluetoothDeviceInquiry::inquiryWithDelegate(Some(self.delegate())) }?;
            unsafe {
                inquiry.setInquiryLength(INQUIRY_SECONDS);
                inquiry.setUpdateNewDeviceNames(true);
            }
            self.inquiry = Some(inquiry);
        }
        self.inquiry.clone()
    }

    fn run_inquiry(&self) -> bool {
        self.inquiry
            .as_ref()
            .is_some_and(|inquiry| unsafe { inquiry.start() } == 0)
    }

    fn stop_scan(&mut self) {
        self.scanning = false;
        if let Some(inquiry) = &self.inquiry {
            unsafe {
                inquiry.stop();
                inquiry.clearFoundDevices();
            }
        }
        self.mark_dirty();
    }

    /// Each inquiry ends after its length; keep going while scanning.
    fn inquiry_complete(&mut self) {
        self.mark_dirty();
        if !self.scanning || self.pairing.is_some() {
            return;
        }
        if !self.run_inquiry() {
            self.scanning = false;
            self.done("adapter", Err("Scanning stopped unexpectedly.".into()));
        }
    }

    // Pairing

    fn pair(&mut self, id: &str) {
        if self.pairing.is_some() {
            return self.done(id, Err("Another device is already pairing.".into()));
        }
        let Some((device, name)) = self.require_device(id) else {
            return;
        };
        if unsafe { device.isPaired() } {
            return self.done(id, Ok(format!("{name} is already paired")));
        }
        let Some(pair) = (unsafe { IOBluetoothDevicePair::pairWithDevice(Some(&device)) }) else {
            return self.done(id, Err(pairing_failed(&name)));
        };
        // Pairing and inquiry fight over the radio; scanning resumes afterwards.
        if let Some(inquiry) = self.inquiry.as_ref().filter(|_| self.scanning) {
            unsafe { inquiry.stop() };
        }
        unsafe { pair.setDelegate(Some(self.delegate())) };
        if unsafe { pair.start() } != 0 {
            return self.done(id, Err(pairing_failed(&name)));
        }
        self.pairing = Some(Pairing {
            id: id.to_string(),
            name,
            pair,
            prompt: None,
            deadline: Instant::now() + PAIR_TIMEOUT,
        });
    }

    fn pairing_prompt(&mut self, prompt: PairPrompt) {
        let Some(pairing) = &mut self.pairing else {
            return;
        };
        pairing.prompt = Some(prompt.clone());
        let device = pairing.id.clone();
        self.send(Event::Pairing { device, prompt });
    }

    fn pair_reply(&mut self, reply: Option<String>) {
        let Some(pairing) = &self.pairing else {
            return;
        };
        match (&pairing.prompt, reply) {
            (Some(PairPrompt::Confirm(_)), Some(_)) => unsafe {
                pairing.pair.replyUserConfirmation(true);
            },
            (Some(PairPrompt::EnterPin), Some(pin)) => reply_pin(&pairing.pair, &pin),
            (_, None) => self.cancel_pairing(),
            _ => {}
        }
    }

    fn cancel_pairing(&mut self) {
        let Some(pairing) = self.pairing.take() else {
            return;
        };
        unsafe {
            if matches!(pairing.prompt, Some(PairPrompt::Confirm(_))) {
                pairing.pair.replyUserConfirmation(false);
            }
            pairing.pair.stop();
        }
        self.after_pairing();
        let message = format!("Pairing with {} was cancelled.", pairing.name);
        self.done(&pairing.id, Err(message));
    }

    fn pairing_finished(&mut self, status: i32) {
        let Some(pairing) = self.pairing.take() else {
            return;
        };
        self.after_pairing();
        if status == 0 {
            self.done(&pairing.id, Ok(format!("Paired with {}", pairing.name)));
        } else {
            self.done(&pairing.id, Err(pairing_failed(&pairing.name)));
        }
    }

    fn poll_pairing(&mut self) {
        let expired = self.pairing.as_ref().map(|p| Instant::now() >= p.deadline);
        if expired != Some(true) {
            return;
        }
        if let Some(pairing) = self.pairing.take() {
            unsafe { pairing.pair.stop() };
            self.after_pairing();
            self.done(&pairing.id, Err(pairing_failed(&pairing.name)));
        }
    }

    fn after_pairing(&mut self) {
        self.mark_dirty();
        if self.scanning && !self.run_inquiry() {
            self.scanning = false;
        }
    }
}

impl Drop for Backend {
    /// Tear down in dependency order: nothing may call the handler after it
    /// is released.
    fn drop(&mut self) {
        if let Some(inquiry) = self.inquiry.take() {
            unsafe {
                inquiry.stop();
                inquiry.setDelegate(None);
            }
        }
        if let Some(pairing) = self.pairing.take() {
            unsafe {
                pairing.pair.stop();
                pairing.pair.setDelegate(None);
            }
        }
        for (_, notification) in self.disconnect_notifications.drain() {
            unsafe { notification.unregister() };
        }
        if let Some(notification) = self.connect_notification.take() {
            unsafe { notification.unregister() };
        }
    }
}

const FORGET_IN_SETTINGS: &str = "Forget this device in System Settings › Bluetooth.";
const SCAN_FAILED: &str = "Couldn't start scanning. Try turning Bluetooth off and on.";

fn unreachable_message(name: &str) -> String {
    format!("{name} didn't respond. Make sure it's nearby and awake.")
}

fn pairing_failed(name: &str) -> String {
    format!("Couldn't pair with {name}. Put it in pairing mode and try again.")
}

fn reply_pin(pair: &IOBluetoothDevicePair, pin: &str) {
    let mut code = BluetoothPINCode { data: [0; 16] };
    let bytes = pin.as_bytes();
    let length = bytes.len().min(code.data.len());
    code.data[..length].copy_from_slice(&bytes[..length]);
    unsafe { pair.replyPINCode_PINCode(length as c_ulong, &mut code) };
}

// IOBluetooth helpers

/// "70-5a-6f-6b-67-f9" or "70:5a:..." to "70:5A:6F:6B:67:F9".
fn normalize_address(address: &str) -> String {
    address.replace('-', ":").to_uppercase()
}

fn address_of(device: &IOBluetoothDevice) -> Option<String> {
    let address = unsafe { device.addressString() }?;
    Some(normalize_address(&address.to_string()))
}

// `name` is declared non-null but IOBluetooth returns nil for unnamed devices.
#[allow(deprecated)]
fn device_name(device: &IOBluetoothDevice) -> String {
    unsafe { device.getName() }
        .map(|name| name.to_string())
        .unwrap_or_default()
}

fn display_name(device: &IOBluetoothDevice, id: &str) -> String {
    let name = device_name(device);
    if name.is_empty() {
        id.to_string()
    } else {
        name
    }
}

fn find_device(id: &str) -> Option<Retained<IOBluetoothDevice>> {
    unsafe { IOBluetoothDevice::deviceWithAddressString(Some(&NSString::from_str(id))) }
}

fn devices_in(array: Option<Retained<NSArray>>) -> Vec<Retained<IOBluetoothDevice>> {
    let Some(array) = array else {
        return Vec::new();
    };
    (0..array.count())
        .filter_map(|index| {
            array
                .objectAtIndex(index)
                .downcast::<IOBluetoothDevice>()
                .ok()
        })
        .collect()
}

/// 0 and 127 mean "no reading".
fn rssi_of(device: &IOBluetoothDevice) -> Option<i16> {
    let rssi = unsafe { device.RSSI() };
    (rssi != 0 && rssi != 127).then_some(i16::from(rssi))
}

fn controller_name() -> String {
    unsafe { IOBluetoothHostController::defaultController() }
        .and_then(|controller| unsafe { controller.nameAsString() })
        .map(|name| name.to_string())
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| "Bluetooth".into())
}
