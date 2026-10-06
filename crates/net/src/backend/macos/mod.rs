//! macOS backend. One thread owns CoreWLAN and the SystemConfiguration dynamic
//! store and runs a CFRunLoop to receive their notifications; it builds every
//! snapshot. Slow work (scans, joins, shelling out) runs on short-lived worker
//! threads that ask that thread for a new snapshot when they finish.

mod interfaces;
mod sc;
mod shell;
mod vpn;
mod wake;
mod wifi;

use super::{Cmd, Event, Rx, Tx};
use crate::model::{Caps, Details, Interface, InterfaceKind, Ipv4Config, Snapshot, Vpn};
use interfaces::WifiBrief;
use objc2_core_foundation::{CFRunLoop, kCFRunLoopDefaultMode};
use sc::Store;
use std::collections::{HashMap, HashSet};
use std::net::{IpAddr, Ipv4Addr};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};
use std::thread;
use std::time::{Duration, Instant};
use wake::Wake;

const NO_WIFI: &str = "This Mac has no Wi-Fi interface.";
/// Gather changes for this long before building a snapshot.
const DEBOUNCE: Duration = Duration::from_millis(30);
/// The first snapshot waits this long for the first VPN poll.
const FIRST_SNAPSHOT_WAIT: Duration = Duration::from_millis(1500);
const VPN_POLL: Duration = Duration::from_secs(5);
/// How long the run loop sleeps when nothing is pending.
const IDLE: Duration = Duration::from_secs(3600);
const SCAN_WAIT: Duration = Duration::from_secs(30);

/// State shared between the run-loop thread, workers and the command task.
struct Shared {
    events: Tx,
    /// Asks the run-loop thread to build a new snapshot.
    wake: Wake,
    /// Set once the command channel closes; every thread then exits.
    stopped: AtomicBool,
    stop_vpn_poll: Sender<()>,
    vpns: Mutex<Vec<Vpn>>,
    vpns_ready: AtomicBool,
    scan: Mutex<Scan>,
    scan_finished: Condvar,
    speeds: Mutex<Speeds>,
}

/// The scan in flight, and how the last one ended so waiters can share it.
struct Scan {
    running: bool,
    finished: u32,
    result: Result<String, String>,
}

/// Wired link speeds, measured once per connection.
#[derive(Default)]
struct Speeds {
    mbps: HashMap<String, u32>,
    requested: HashSet<String>,
}

impl Shared {
    fn refresh(&self) {
        self.wake.send();
    }

    fn stop(&self) {
        self.stopped.store(true, Ordering::SeqCst);
        let _ = self.stop_vpn_poll.send(());
        self.wake.send();
    }

    fn done(&self, target: &str, result: Result<String, String>) {
        let _ = self.events.send(Event::Done {
            target: target.to_string(),
            result,
        });
    }
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

pub fn spawn(mut cmds: Rx, events: Tx) {
    let (wake, wanted) = Wake::new();
    let (stop_vpn_poll, vpn_stopped) = mpsc::channel();
    let shared = Arc::new(Shared {
        events,
        wake: wake.clone(),
        stopped: AtomicBool::new(false),
        stop_vpn_poll,
        vpns: Mutex::new(Vec::new()),
        vpns_ready: AtomicBool::new(false),
        scan: Mutex::new(Scan {
            running: false,
            finished: 0,
            result: Ok(String::new()),
        }),
        scan_finished: Condvar::new(),
        speeds: Mutex::new(Speeds::default()),
    });
    let s = shared.clone();
    thread::spawn(move || run_loop(&s, wake, wanted));
    let s = shared.clone();
    thread::spawn(move || poll_vpns(&s, &vpn_stopped));
    let s = shared.clone();
    // The popup just opened: show cached results now, fresh ones when ready.
    thread::spawn(move || {
        let _ = scan(&s);
    });
    tokio::spawn(async move {
        while let Some(cmd) = cmds.recv().await {
            let shared = shared.clone();
            thread::spawn(move || handle(cmd, &shared));
        }
        shared.stop();
    });
}

// The run-loop thread

fn run_loop(shared: &Arc<Shared>, wake: Wake, wanted: Receiver<()>) {
    wake.attach();
    let store = match Store::watcher(wake.clone()) {
        Ok(store) => store,
        Err(message) => return shared.done("backend", Err(message)),
    };
    let radio = wifi::Radio::start(wake);
    let started = Instant::now();
    let mut dirty_since = Some(started);
    let mut sent_first = false;
    let mut reported = None;
    while !shared.stopped.load(Ordering::SeqCst) {
        while wanted.try_recv().is_ok() {
            dirty_since.get_or_insert_with(Instant::now);
        }
        let ready = sent_first
            || shared.vpns_ready.load(Ordering::SeqCst)
            || started.elapsed() > FIRST_SNAPSHOT_WAIT;
        if dirty_since.is_some_and(|since| ready && since.elapsed() >= DEBOUNCE) {
            dirty_since = None;
            sent_first = true;
            let (snapshot, problem) = snapshot(shared, &store, radio.as_ref());
            if problem != reported {
                if let Some(message) = &problem {
                    shared.done("backend", Err(message.clone()));
                }
                reported = problem;
            }
            let _ = shared.events.send(Event::Snapshot(snapshot));
        }
        let wake_at = match dirty_since {
            Some(_) if !ready => started + FIRST_SNAPSHOT_WAIT,
            Some(since) => since + DEBOUNCE,
            None => Instant::now() + IDLE,
        };
        let sleep = wake_at.saturating_duration_since(Instant::now());
        // Any handled event returns, so a wake-up or notification re-checks the state above.
        CFRunLoop::run_in_mode(unsafe { kCFRunLoopDefaultMode }, sleep.as_secs_f64(), true);
    }
}

/// Build a snapshot, plus a problem to tell the user about if the network
/// settings couldn't be read.
fn snapshot(
    shared: &Arc<Shared>,
    store: &Store,
    radio: Option<&wifi::Radio>,
) -> (Snapshot, Option<String>) {
    let record = radio.and_then(|r| {
        let key = format!("State:/Network/Interface/{}/AirPort", r.device);
        store
            .data(&key, "CachedScanRecord")
            .and_then(|data| wifi::scan_record(&data))
    });
    let current = radio.and_then(|r| r.current(record));
    let scanning = lock(&shared.scan).running;
    let wifi_state = radio.map(|r| r.read(current.clone(), scanning));
    let brief = WifiBrief {
        power: wifi_state.as_ref().is_some_and(|w| w.power),
        ssid: current.and_then(|c| c.ssid),
    };
    let (services, problem) = match interfaces::services(store) {
        Ok(services) => (services, None),
        Err(message) => (vec![], Some(message)),
    };
    let speeds = lock(&shared.speeds).mbps.clone();
    let built = interfaces::build(store, &services, &brief, &speeds);
    measure_link_speeds(shared, &built.interfaces);
    let snapshot = Snapshot {
        interfaces: built.interfaces,
        wifi: wifi_state,
        vpns: lock(&shared.vpns).clone(),
        primary: built.primary,
        caps: Caps {
            edit_ip: true,
            edit_needs_admin: true,
        },
    };
    (snapshot, problem)
}

/// Wired speeds come from `ifconfig`, so measure each connection once, off-thread.
fn measure_link_speeds(shared: &Arc<Shared>, rows: &[Interface]) {
    let mut speeds = lock(&shared.speeds);
    for row in rows.iter().filter(|r| r.kind != InterfaceKind::Wifi) {
        if !row.connected {
            speeds.mbps.remove(&row.device);
            speeds.requested.remove(&row.device);
        } else if speeds.requested.insert(row.device.clone()) {
            let (shared, device) = (shared.clone(), row.device.clone());
            thread::spawn(move || {
                if let Some(mbps) = interfaces::link_speed(&device) {
                    lock(&shared.speeds).mbps.insert(device, mbps);
                    shared.refresh();
                }
            });
        }
    }
}

// VPN polling

fn poll_vpns(shared: &Shared, stopped: &Receiver<()>) {
    loop {
        update_vpns(shared);
        // Anything on the channel, or its closing, means stop.
        if stopped.recv_timeout(VPN_POLL) != Err(RecvTimeoutError::Timeout) {
            return;
        }
    }
}

fn update_vpns(shared: &Shared) {
    let polled = vpn::poll();
    let changed = {
        let mut current = lock(&shared.vpns);
        let changed = json(&current) != json(&polled);
        *current = polled;
        changed
    };
    let first = !shared.vpns_ready.swap(true, Ordering::SeqCst);
    if changed || first {
        shared.refresh();
    }
}

fn json(vpns: &[Vpn]) -> String {
    serde_json::to_string(vpns).unwrap_or_default()
}

// Commands. Each runs on its own worker thread and ends in a `Done`.

fn handle(cmd: Cmd, shared: &Arc<Shared>) {
    match cmd {
        Cmd::Rescan => shared.done("wifi", scan(shared)),
        Cmd::Join { network, password } => {
            let result = join(&network, password.as_deref());
            shared.refresh();
            shared.done(&network, result);
        }
        Cmd::Forget { ssid } => {
            let result = forget(&ssid);
            shared.refresh();
            shared.done(&ssid, result);
        }
        Cmd::SetWifiPower(on) => {
            let result = set_power(on);
            shared.refresh();
            shared.done("wifi", result);
        }
        Cmd::SetIpv4 { interface, config } => {
            let result = set_ipv4(&interface, &config);
            shared.refresh();
            shared.done(&interface, result);
        }
        Cmd::SetVpn { vpn, on } => {
            let result = vpn::set(&vpn, on);
            update_vpns(shared);
            shared.done(&vpn, result);
        }
        Cmd::Details => {
            let _ = shared.events.send(Event::Details(details()));
        }
        Cmd::RevealPassword { ssid } => {
            let password = wifi::reveal_password(&ssid);
            let _ = shared.events.send(Event::Password { ssid, password });
        }
        Cmd::RequestLocation => {
            let result = request_location();
            shared.done("wifi", result);
            let _ = scan(shared);
        }
    }
}

fn wifi_device() -> Result<String, String> {
    wifi::default_device().ok_or_else(|| NO_WIFI.to_string())
}

/// Run a fresh scan, showing the scanning state while it runs. A scan that
/// is already running is shared: waiters get its result.
fn scan(shared: &Shared) -> Result<String, String> {
    let mut state = lock(&shared.scan);
    if state.running {
        let before = state.finished;
        let (state, timeout) = shared
            .scan_finished
            .wait_timeout_while(state, SCAN_WAIT, |s| s.finished == before)
            .unwrap_or_else(PoisonError::into_inner);
        if timeout.timed_out() {
            return Err("The scan is taking too long. Try again in a moment.".to_string());
        }
        return state.result.clone();
    }
    state.running = true;
    drop(state);
    shared.refresh();
    let result = wifi_device()
        .and_then(|device| wifi::scan(&device))
        .map(|()| "Scan finished.".to_string());
    let mut state = lock(&shared.scan);
    state.running = false;
    state.finished = state.finished.wrapping_add(1);
    state.result = result.clone();
    drop(state);
    shared.scan_finished.notify_all();
    shared.refresh();
    result
}

fn join(network: &str, password: Option<&str>) -> Result<String, String> {
    wifi::join(&wifi_device()?, network, password)
}

fn forget(ssid: &str) -> Result<String, String> {
    wifi::forget(&wifi_device()?, ssid)?;
    Ok(format!("Forgot {ssid}."))
}

fn set_power(on: bool) -> Result<String, String> {
    wifi::set_power(&wifi_device()?, on)?;
    Ok(format!("Wi-Fi is {}.", if on { "on" } else { "off" }))
}

fn set_ipv4(interface: &str, config: &Ipv4Config) -> Result<String, String> {
    let service = interfaces::services(&Store::reader()?)?
        .into_iter()
        .find(|s| s.bsd == interface)
        .ok_or_else(|| format!("{interface} isn't a network service any more."))?;
    shell::admin(&ipv4_commands(&service.name, config)?)?;
    Ok("Settings saved.".to_string())
}

/// The `networksetup` calls that apply a config. Addresses are validated so
/// nothing odd reaches the privileged shell.
fn ipv4_commands(service: &str, config: &Ipv4Config) -> Result<Vec<Vec<String>>, String> {
    let networksetup = |args: &[&str]| {
        let mut command = vec!["/usr/sbin/networksetup".to_string()];
        command.extend(args.iter().map(|a| a.to_string()));
        command
    };
    let mut commands = Vec::new();
    match &config.manual {
        Some(m) => {
            let address = ipv4_text(&m.address, "IP address")?;
            let subnet = ipv4_text(&m.subnet, "subnet mask")?;
            let mut args = vec!["-setmanual", service, &address, &subnet];
            let router;
            if !m.router.is_empty() {
                router = ipv4_text(&m.router, "router")?;
                args.push(&router);
            }
            commands.push(networksetup(&args));
        }
        None => commands.push(networksetup(&["-setdhcp", service])),
    }
    let mut dns = vec!["-setdnsservers", service];
    let servers = config
        .dns
        .iter()
        .map(|d| {
            d.parse::<IpAddr>()
                .map(|ip| ip.to_string())
                .map_err(|_| format!("{d} isn't a valid DNS server address."))
        })
        .collect::<Result<Vec<_>, _>>()?;
    dns.extend(servers.iter().map(String::as_str));
    if servers.is_empty() {
        dns.push("Empty");
    }
    commands.push(networksetup(&dns));
    Ok(commands)
}

fn ipv4_text(text: &str, what: &str) -> Result<String, String> {
    text.trim()
        .parse::<Ipv4Addr>()
        .map(|ip| ip.to_string())
        .map_err(|_| format!("{text} isn't a valid {what}."))
}

fn details() -> Details {
    let mut details = wifi_device().map(|d| wifi::details(&d)).unwrap_or_default();
    details.public_ip = public_ip();
    details
}

/// Cloudflare's meta endpoint reports the address we connect from.
fn public_ip() -> Option<String> {
    let args = [
        "-s",
        "-m",
        "5",
        "-H",
        "Referer: https://speed.cloudflare.com/",
        "https://speed.cloudflare.com/meta",
    ];
    let out = shell::run("/usr/bin/curl", &args, Duration::from_secs(7)).ok()?;
    let meta: serde_json::Value = serde_json::from_str(&out.stdout).ok()?;
    meta.get("clientIp")?.as_str().map(str::to_string)
}

fn request_location() -> Result<String, String> {
    const MISSING: &str = "Open Telmo.app to allow Location.";
    let args = ["host", "request-location"];
    let out =
        shell::run("telmo", &args, Duration::from_secs(60)).map_err(|_| MISSING.to_string())?;
    if out.ok {
        Ok("Location requested.".to_string())
    } else {
        Err(MISSING.to_string())
    }
}

/// These touch the real system (a scan takes a few seconds); run them with
/// `cargo test -p telmo-net -- --ignored --nocapture`.
#[cfg(test)]
mod live {
    use super::*;

    #[test]
    #[ignore]
    fn rescan_and_details() {
        let device = wifi_device().expect("wifi device");
        let started = Instant::now();
        wifi::scan(&device).expect("scan");
        println!("scan took {:?}", started.elapsed());
        println!("{:?}", details());
    }

    #[tokio::test(flavor = "current_thread")]
    #[ignore]
    async fn backend_rescan_ends_in_done_and_a_snapshot() {
        use tokio::sync::mpsc::unbounded_channel;
        use tokio::time::timeout;
        let (cmd_tx, cmd_rx) = unbounded_channel();
        let (event_tx, mut events) = unbounded_channel();
        spawn(cmd_rx, event_tx);
        cmd_tx.send(Cmd::Rescan).expect("send");
        let (mut snapshots, mut idle_snapshot, mut done) = (0, false, false);
        let wait = async {
            while let Some(event) = events.recv().await {
                match event {
                    Event::Snapshot(s) => {
                        snapshots += 1;
                        idle_snapshot |= s.wifi.is_some_and(|w| !w.scanning);
                    }
                    Event::Done { target, result } => {
                        println!("done {target}: {result:?}");
                        done |= target == "wifi";
                    }
                    _ => {}
                }
                if done && idle_snapshot {
                    break;
                }
            }
        };
        timeout(Duration::from_secs(20), wait)
            .await
            .expect("timed out");
        println!("{snapshots} snapshots");
    }

    #[test]
    #[ignore]
    fn core_wlan_events_reach_the_run_loop() {
        let (wake, rx) = Wake::new();
        let _radio = wifi::Radio::start(wake).expect("radio");
        let device = wifi_device().expect("device");
        thread::spawn(move || {
            let _ = wifi::scan(&device);
        });
        let end = Instant::now() + Duration::from_secs(12);
        let mut events = 0;
        while Instant::now() < end {
            CFRunLoop::run_in_mode(unsafe { kCFRunLoopDefaultMode }, 0.1, false);
            while rx.try_recv().is_ok() {
                events += 1;
            }
        }
        println!("{events} CoreWLAN events");
    }

    #[test]
    #[ignore]
    fn ipv4_commands_for_manual_config() {
        use crate::model::ManualIpv4;
        let config = Ipv4Config {
            manual: Some(ManualIpv4 {
                address: "192.168.1.50".to_string(),
                subnet: "255.255.255.0".to_string(),
                router: "192.168.1.1".to_string(),
            }),
            dns: vec![],
        };
        println!("{:?}", ipv4_commands("Wi-Fi", &config));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::ManualIpv4;

    #[test]
    fn builds_manual_and_dhcp_commands() {
        let manual = Ipv4Config {
            manual: Some(ManualIpv4 {
                address: "10.0.0.5".to_string(),
                subnet: "255.255.255.0".to_string(),
                router: "10.0.0.1".to_string(),
            }),
            dns: vec!["1.1.1.1".to_string()],
        };
        let commands = ipv4_commands("USB LAN", &manual).expect("valid");
        assert_eq!(
            commands[0][1..],
            [
                "-setmanual",
                "USB LAN",
                "10.0.0.5",
                "255.255.255.0",
                "10.0.0.1"
            ]
        );
        assert_eq!(commands[1][1..], ["-setdnsservers", "USB LAN", "1.1.1.1"]);
        let auto = Ipv4Config {
            manual: None,
            dns: vec![],
        };
        let commands = ipv4_commands("USB LAN", &auto).expect("valid");
        assert_eq!(commands[0][1..], ["-setdhcp", "USB LAN"]);
        assert_eq!(commands[1][1..], ["-setdnsservers", "USB LAN", "Empty"]);
    }

    #[test]
    fn rejects_bad_addresses() {
        let bad = Ipv4Config {
            manual: Some(ManualIpv4 {
                address: "1.2.3.4; rm -rf /".to_string(),
                subnet: "255.255.255.0".to_string(),
                router: String::new(),
            }),
            dns: vec![],
        };
        assert!(ipv4_commands("Wi-Fi", &bad).is_err());
    }
}
