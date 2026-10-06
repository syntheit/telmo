//! BlueZ backend, talking to bluetoothd over D-Bus with `bluer`.

use super::{Cmd, Event, Rx, Tx};
use crate::model::{Adapter, Battery, Device, Kind, PairPrompt, Snapshot};
use bluer::{
    AdapterEvent, Address, ErrorKind, Session,
    agent::{Agent, AgentHandle, ReqError, ReqResult},
};
use futures_util::StreamExt;
use std::{
    collections::HashMap,
    future::pending,
    str::FromStr,
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::{
    select,
    sync::{
        mpsc::{UnboundedSender, unbounded_channel},
        oneshot,
    },
    task::JoinHandle,
    time::{sleep, timeout},
};

const DEBOUNCE: Duration = Duration::from_millis(30);
/// Unregistering the agent finishes in the background shortly after it is dropped.
const AGENT_UNREGISTER: Duration = Duration::from_millis(200);
const RETRY: Duration = Duration::from_secs(5);
const PROMPT_TIMEOUT: Duration = Duration::from_secs(60);

const NO_ADAPTER: &str =
    "No Bluetooth adapter found. Plug one in or check that the Bluetooth service is running.";

type BtAdapter = bluer::Adapter;
type Reply = oneshot::Sender<Option<String>>;
type Changed = UnboundedSender<()>;

pub fn spawn(cmds: Rx, events: Tx) -> JoinHandle<()> {
    tokio::spawn(run(cmds, events))
}

async fn run(mut cmds: Rx, events: Tx) {
    let prompter = Prompter::new(events.clone());
    loop {
        match connect(&prompter).await {
            Ok((_session, adapter, agent, pairable)) => {
                let keep_going = serve(&adapter, pairable, &mut cmds, &events, &prompter).await;
                drop(agent);
                // BlueZ turns Pairable on when our agent goes away, so put it back afterwards.
                sleep(AGENT_UNREGISTER).await;
                let _ = adapter.set_pairable(pairable).await;
                if !keep_going {
                    return;
                }
            }
            Err(_) => {
                if events.send(Event::Snapshot(Snapshot::default())).is_err() {
                    return;
                }
            }
        }
        if !wait_for_adapter(&mut cmds, &events).await {
            return;
        }
    }
}

/// The session and agent registration must stay alive as long as the adapter is used.
/// Also returns the adapter's Pairable value from before BlueZ turned it on for our agent.
async fn connect(prompter: &Prompter) -> bluer::Result<(Session, BtAdapter, AgentHandle, bool)> {
    let session = Session::new().await?;
    let adapter = session.default_adapter().await?;
    let pairable = adapter.is_pairable().await.unwrap_or(false);
    let agent = session.register_agent(prompter.agent()).await?;
    Ok((session, adapter, agent, pairable))
}

/// Answer commands with an error until the retry delay is over.
/// Returns false when the UI is gone.
async fn wait_for_adapter(cmds: &mut Rx, events: &Tx) -> bool {
    let retry = sleep(RETRY);
    tokio::pin!(retry);
    loop {
        select! {
            () = &mut retry => return true,
            cmd = cmds.recv() => {
                let Some(cmd) = cmd else { return false };
                if let Some(target) = target(&cmd) {
                    done(events, &target, Err(NO_ADAPTER.to_string()));
                }
            }
        }
    }
}

/// The device id a command acts on, "adapter" for adapter-wide ones.
fn target(cmd: &Cmd) -> Option<String> {
    match cmd {
        Cmd::SetPower(_) | Cmd::StartScan | Cmd::StopScan => Some("adapter".into()),
        Cmd::Connect(id)
        | Cmd::Disconnect(id)
        | Cmd::Pair(id)
        | Cmd::SetTrusted(id, _)
        | Cmd::Forget(id)
        | Cmd::Rename(id, _) => Some(id.clone()),
        Cmd::PairReply(_) => None,
    }
}

/// Runs while the adapter exists. `pairable` is what StopScan puts Pairable back to. Returns false when the UI is gone.
async fn serve(
    adapter: &BtAdapter,
    pairable: bool,
    cmds: &mut Rx,
    events: &Tx,
    prompter: &Prompter,
) -> bool {
    let Ok(adapter_events) = adapter.events().await else {
        return true;
    };
    let mut adapter_events = Box::pin(adapter_events);
    let (changed_tx, mut changed_rx) = unbounded_channel();
    let mut watchers: HashMap<Address, JoinHandle<()>> = HashMap::new();
    for address in adapter.device_addresses().await.unwrap_or_default() {
        watchers.insert(address, watch_device(adapter, address, &changed_tx));
    }
    let mut scan: Option<JoinHandle<()>> = None;
    let _ = changed_tx.send(());

    let keep_going = loop {
        select! {
            event = adapter_events.next() => match event {
                None => break true,
                Some(AdapterEvent::DeviceAdded(address)) => {
                    watchers
                        .entry(address)
                        .or_insert_with(|| watch_device(adapter, address, &changed_tx));
                    let _ = changed_tx.send(());
                }
                Some(AdapterEvent::DeviceRemoved(address)) => {
                    if let Some(task) = watchers.remove(&address) {
                        task.abort();
                    }
                    let _ = changed_tx.send(());
                }
                Some(AdapterEvent::PropertyChanged(_)) => {
                    let _ = changed_tx.send(());
                }
            },
            Some(()) = changed_rx.recv() => {
                sleep(DEBOUNCE).await;
                while changed_rx.try_recv().is_ok() {}
                if events.send(Event::Snapshot(snapshot(adapter).await)).is_err() {
                    break false;
                }
            }
            cmd = cmds.recv() => {
                let Some(cmd) = cmd else { break false };
                handle(adapter, pairable, events, prompter, &mut scan, cmd).await;
            }
        }
    };
    for task in watchers.into_values() {
        task.abort();
    }
    if let Some(task) = scan {
        task.abort();
    }
    keep_going
}

/// Ping `changed` on every property change of one device.
fn watch_device(adapter: &BtAdapter, address: Address, changed: &Changed) -> JoinHandle<()> {
    let device = adapter.device(address);
    let changed = changed.clone();
    tokio::spawn(async move {
        let Ok(device) = device else { return };
        let Ok(mut stream) = device.events().await else {
            return;
        };
        while stream.next().await.is_some() {
            if changed.send(()).is_err() {
                return;
            }
        }
    })
}

async fn handle(
    adapter: &BtAdapter,
    pairable: bool,
    events: &Tx,
    prompter: &Prompter,
    scan: &mut Option<JoinHandle<()>>,
    cmd: Cmd,
) {
    match cmd {
        Cmd::StartScan => start_scan(adapter, events, scan).await,
        Cmd::StopScan => {
            stop_scan(adapter, scan.take(), pairable).await;
            done(events, "adapter", Ok("Stopped scanning".into()));
        }
        Cmd::PairReply(reply) => prompter.reply(reply),
        cmd => {
            let Some(target) = target(&cmd) else { return };
            let adapter = adapter.clone();
            let events = events.clone();
            let prompter = prompter.clone();
            // Pairing waits for the user, so commands must not block each other.
            tokio::spawn(async move {
                let result = run_cmd(&adapter, &prompter, cmd).await;
                done(&events, &target, result);
            });
        }
    }
}

fn done(events: &Tx, target: &str, result: Result<String, String>) {
    let _ = events.send(Event::Done {
        target: target.into(),
        result,
    });
}

async fn start_scan(adapter: &BtAdapter, events: &Tx, scan: &mut Option<JoinHandle<()>>) {
    if scan.as_ref().is_some_and(|task| !task.is_finished()) {
        return done(events, "adapter", Ok("Already scanning".into()));
    }
    // Devices may want to pair back while we look around.
    let _ = adapter.set_pairable(true).await;
    let adapter = adapter.clone();
    let events = events.clone();
    let task = tokio::spawn(async move {
        match adapter.discover_devices().await {
            Ok(stream) => {
                done(&events, "adapter", Ok("Scanning for devices".into()));
                // Discovery stops when the stream is dropped, which happens on abort.
                let _stream = stream;
                pending::<()>().await;
            }
            Err(e) => done(&events, "adapter", Err(adapter_error("scan", &e))),
        }
    });
    *scan = Some(task);
}

/// Stops scanning, if we are, and puts Pairable back to `pairable`.
async fn stop_scan(adapter: &BtAdapter, scan: Option<JoinHandle<()>>, pairable: bool) {
    if let Some(task) = scan {
        task.abort();
        let _ = task.await;
    }
    let _ = adapter.set_pairable(pairable).await;
}

async fn run_cmd(adapter: &BtAdapter, prompter: &Prompter, cmd: Cmd) -> Result<String, String> {
    match cmd {
        Cmd::SetPower(on) => {
            adapter
                .set_powered(on)
                .await
                .map_err(|e| adapter_error("change Bluetooth power", &e))?;
            Ok(if on {
                "Bluetooth is on"
            } else {
                "Bluetooth is off"
            }
            .into())
        }
        Cmd::Connect(id) => connect_device(adapter, &id).await,
        Cmd::Disconnect(id) => {
            let (device, name) = open(adapter, &id).await?;
            device
                .disconnect()
                .await
                .map_err(|e| device_error(&name, "disconnect from", &e))?;
            Ok(format!("Disconnected from {name}"))
        }
        Cmd::Pair(id) => pair_device(adapter, prompter, &id).await,
        Cmd::SetTrusted(id, trusted) => {
            let (device, name) = open(adapter, &id).await?;
            device
                .set_trusted(trusted)
                .await
                .map_err(|e| device_error(&name, "change trust for", &e))?;
            Ok(if trusted {
                format!("{name} is trusted and will reconnect on its own")
            } else {
                format!("{name} is no longer trusted")
            })
        }
        Cmd::Forget(id) => {
            let (_, name) = open(adapter, &id).await?;
            adapter
                .remove_device(parse(&id)?)
                .await
                .map_err(|e| device_error(&name, "forget", &e))?;
            Ok(format!("Forgot {name}"))
        }
        Cmd::Rename(id, new_name) => {
            let (device, name) = open(adapter, &id).await?;
            device
                .set_alias(new_name.clone())
                .await
                .map_err(|e| device_error(&name, "rename", &e))?;
            Ok(format!("Renamed to {new_name}"))
        }
        // Handled by the main loop.
        Cmd::StartScan | Cmd::StopScan | Cmd::PairReply(_) => Ok(String::new()),
    }
}

async fn connect_device(adapter: &BtAdapter, id: &str) -> Result<String, String> {
    let (device, name) = open(adapter, id).await?;
    match device.connect().await {
        Ok(()) => Ok(format!("Connected to {name}")),
        Err(e) if e.kind == ErrorKind::AlreadyConnected => {
            Ok(format!("{name} is already connected"))
        }
        Err(e) => Err(device_error(&name, "connect to", &e)),
    }
}

async fn pair_device(adapter: &BtAdapter, prompter: &Prompter, id: &str) -> Result<String, String> {
    let (device, name) = open(adapter, id).await?;
    prompter.set_pairing(Some(parse(id)?));
    let paired = device.pair().await;
    prompter.set_pairing(None);
    paired.map_err(|e| device_error(&name, "pair with", &e))?;

    device
        .set_trusted(true)
        .await
        .map_err(|e| device_error(&name, "trust", &e))?;
    match device.connect().await {
        Ok(()) => Ok(format!("Paired and connected to {name}")),
        Err(_) => Ok(format!(
            "Paired with {name}, but it did not connect. Try Connect again."
        )),
    }
}

fn parse(id: &str) -> Result<Address, String> {
    Address::from_str(id).map_err(|_| format!("{id} is not a valid Bluetooth address."))
}

/// The device plus a name to use in messages.
async fn open(adapter: &BtAdapter, id: &str) -> Result<(bluer::Device, String), String> {
    let device = adapter
        .device(parse(id)?)
        .map_err(|_| format!("{id} is not a valid Bluetooth address."))?;
    let name = device.alias().await.unwrap_or_else(|_| id.to_string());
    Ok((device, name))
}

fn adapter_error(action: &str, e: &bluer::Error) -> String {
    match e.kind {
        ErrorKind::NotReady | ErrorKind::NotAvailable => {
            "Bluetooth is turned off. Turn it on and try again.".into()
        }
        ErrorKind::InProgress => "Bluetooth is busy. Try again in a moment.".into(),
        ErrorKind::NotPermitted | ErrorKind::NotAuthorized => format!(
            "Bluetooth refused to {action}. Check that you are allowed to manage Bluetooth."
        ),
        _ => format!("Could not {action}: {}.", e.message.trim_end_matches('.')),
    }
}

fn device_error(name: &str, action: &str, e: &bluer::Error) -> String {
    match e.kind {
        ErrorKind::ConnectionAttemptFailed | ErrorKind::Failed => {
            format!("{name} didn't respond. Make sure it's on and nearby.")
        }
        ErrorKind::AuthenticationFailed
        | ErrorKind::AuthenticationRejected
        | ErrorKind::AuthenticationCanceled => {
            format!("{name} refused to pair. Put it in pairing mode and try again.")
        }
        ErrorKind::AuthenticationTimeout => {
            format!("{name} didn't answer the pairing request in time. Try again.")
        }
        ErrorKind::DoesNotExist => format!("{name} is no longer available."),
        _ => adapter_error(&format!("{action} {name}"), e),
    }
}

async fn snapshot(adapter: &BtAdapter) -> Snapshot {
    let powered = adapter.is_powered().await.unwrap_or(false);
    let discovering = adapter.is_discovering().await.unwrap_or(false);
    let mut devices = Vec::new();
    for address in adapter.device_addresses().await.unwrap_or_default() {
        if let Some(device) = read_device(adapter, address).await
            && (device.paired || discovering)
        {
            devices.push(device);
        }
    }
    Snapshot {
        adapter: Some(Adapter {
            name: adapter.name().to_string(),
            powered,
            discovering,
        }),
        devices,
    }
}

/// None when the device vanished while we were reading it.
async fn read_device(adapter: &BtAdapter, address: Address) -> Option<Device> {
    let device = adapter.device(address).ok()?;
    let name = display_name(&device, address).await?;
    let icon = device.icon().await.ok().flatten();
    Some(Device {
        id: address.to_string(),
        name,
        kind: kind(icon.as_deref()),
        paired: device.is_paired().await.unwrap_or(false),
        connected: device.is_connected().await.unwrap_or(false),
        trusted: device.is_trusted().await.unwrap_or(false),
        battery: device
            .battery_percentage()
            .await
            .ok()
            .flatten()
            .map(Battery::Single),
        rssi: device.rssi().await.ok().flatten(),
    })
}

/// BlueZ reports the address as the alias of a device without a name.
async fn display_name(device: &bluer::Device, address: Address) -> Option<String> {
    let alias = device.alias().await.ok()?;
    let is_address = alias
        .replace('-', ":")
        .eq_ignore_ascii_case(&address.to_string());
    Some(if is_address {
        "Unnamed device".into()
    } else {
        alias
    })
}

fn kind(icon: Option<&str>) -> Kind {
    match icon {
        Some("audio-headset" | "audio-headphones") => Kind::Headphones,
        Some("audio-card") => Kind::Speaker,
        Some("input-keyboard") => Kind::Keyboard,
        Some("input-mouse") => Kind::Mouse,
        Some("input-gaming") => Kind::Gamepad,
        Some("phone") => Kind::Phone,
        Some("computer") => Kind::Computer,
        _ => Kind::Other,
    }
}

/// Bridges BlueZ's pairing agent to the UI: asks through `Event::Pairing`,
/// waits for the answer that arrives as `Cmd::PairReply`.
#[derive(Clone)]
struct Prompter {
    events: Tx,
    waiting: Arc<Mutex<Option<Reply>>>,
    /// The device we are pairing with right now, if any.
    pairing: Arc<Mutex<Option<Address>>>,
}

impl Prompter {
    fn new(events: Tx) -> Self {
        Self {
            events,
            waiting: Default::default(),
            pairing: Default::default(),
        }
    }

    fn set_pairing(&self, address: Option<Address>) {
        if let Ok(mut pairing) = self.pairing.lock() {
            *pairing = address;
        }
    }

    fn is_pairing(&self, address: Address) -> bool {
        self.pairing.lock().is_ok_and(|p| *p == Some(address))
    }

    fn reply(&self, reply: Option<String>) {
        let sender = self.waiting.lock().ok().and_then(|mut w| w.take());
        if let Some(sender) = sender {
            let _ = sender.send(reply);
        }
    }

    fn prompt(&self, device: Address, prompt: PairPrompt) -> oneshot::Receiver<Option<String>> {
        let (tx, rx) = oneshot::channel();
        if let Ok(mut waiting) = self.waiting.lock() {
            *waiting = Some(tx);
        }
        let device = device.to_string();
        let _ = self.events.send(Event::Pairing { device, prompt });
        rx
    }

    /// Wait for the user's answer. None means rejected, cancelled or too slow.
    async fn ask(&self, device: Address, prompt: PairPrompt) -> Option<String> {
        let rx = self.prompt(device, prompt);
        timeout(PROMPT_TIMEOUT, rx).await.ok()?.ok()?
    }

    async fn ask_pin(&self, device: Address) -> ReqResult<String> {
        self.ask(device, PairPrompt::EnterPin)
            .await
            .filter(|pin| !pin.is_empty())
            .ok_or(ReqError::Rejected)
    }

    /// Show a code until BlueZ says to stop or the user gives up.
    async fn show(
        &self,
        device: Address,
        code: u32,
        cancel: oneshot::Receiver<()>,
    ) -> ReqResult<()> {
        let rx = self.prompt(device, PairPrompt::DisplayPasskey(code));
        select! {
            reply = rx => match reply {
                Ok(Some(_)) => Ok(()),
                _ => Err(ReqError::Rejected),
            },
            _ = cancel => Ok(()),
            () = sleep(PROMPT_TIMEOUT) => Err(ReqError::Rejected),
        }
    }

    fn agent(&self) -> Agent {
        let confirm = self.clone();
        let passkey = self.clone();
        let pin = self.clone();
        let show_passkey = self.clone();
        let show_pin = self.clone();
        let service = self.clone();
        Agent {
            request_default: true,
            request_confirmation: Some(Box::new(move |req| {
                let p = confirm.clone();
                Box::pin(async move {
                    match p.ask(req.device, PairPrompt::Confirm(req.passkey)).await {
                        Some(_) => Ok(()),
                        None => Err(ReqError::Rejected),
                    }
                })
            })),
            request_passkey: Some(Box::new(move |req| {
                let p = passkey.clone();
                Box::pin(async move {
                    let pin = p.ask_pin(req.device).await?;
                    pin.trim().parse().map_err(|_| ReqError::Rejected)
                })
            })),
            request_pin_code: Some(Box::new(move |req| {
                let p = pin.clone();
                Box::pin(async move { p.ask_pin(req.device).await })
            })),
            display_passkey: Some(Box::new(move |req| {
                let p = show_passkey.clone();
                Box::pin(async move {
                    // Later calls only report typing progress on the device.
                    if req.entered > 0 {
                        return Ok(());
                    }
                    p.show(req.device, req.passkey, req.cancel).await
                })
            })),
            display_pin_code: Some(Box::new(move |req| {
                let p = show_pin.clone();
                Box::pin(async move {
                    let code = req.pincode.parse().map_err(|_| ReqError::Rejected)?;
                    p.show(req.device, code, req.cancel).await
                })
            })),
            authorize_service: Some(Box::new(move |req| {
                let p = service.clone();
                Box::pin(async move {
                    // Only while we pair; trusted devices skip this question.
                    if p.is_pairing(req.device) {
                        Ok(())
                    } else {
                        Err(ReqError::Rejected)
                    }
                })
            })),
            ..Default::default()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Needs a real adapter: `cargo test -p telmo-bt -- --ignored --nocapture`.
    #[tokio::test(flavor = "current_thread")]
    #[ignore]
    async fn scan_finds_devices() {
        let (cmd_tx, cmd_rx) = unbounded_channel();
        let (event_tx, mut event_rx) = unbounded_channel();
        let _backend = spawn(cmd_rx, event_tx);
        cmd_tx.send(Cmd::StartScan).unwrap();

        let mut last = Snapshot::default();
        let deadline = sleep(Duration::from_secs(12));
        tokio::pin!(deadline);
        loop {
            select! {
                () = &mut deadline => break,
                Some(event) = event_rx.recv() => match event {
                    Event::Snapshot(s) => last = s,
                    other => println!("{other:?}"),
                },
            }
        }
        println!("while scanning: {last:#?}");
        cmd_tx.send(Cmd::StopScan).unwrap();
        while let Ok(Some(event)) = timeout(Duration::from_secs(2), event_rx.recv()).await {
            if let Event::Snapshot(s) = event {
                last = s;
            }
        }
        assert!(last.adapter.is_some_and(|a| !a.discovering));
    }
}
