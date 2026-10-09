//! PulseAudio protocol backend (pipewire-pulse speaks it). libpulse runs its
//! own threaded mainloop; this thread sleeps until a command, a server change
//! or a connection change arrives, and touches libpulse only under its lock.

use super::{Cmd, Event, Rx, Tx};
use crate::model::{Caps, Device, Direction, Profile, Snapshot, Stream, Target};
use libpulse_binding::{
    callbacks::ListResult,
    context::{
        Context, FlagSet, State,
        introspect::{CardInfo, SinkInfo, SinkInputInfo, SourceInfo},
        subscribe::InterestMaskSet,
    },
    mainloop::threaded::Mainloop,
    proplist::Proplist,
    volume::{ChannelVolumes, Volume},
};
use std::{cell::RefCell, rc::Rc, thread, time::Duration};
use tokio::sync::mpsc::{UnboundedSender, unbounded_channel};

const DEBOUNCE: Duration = Duration::from_millis(30);
const RETRY: Duration = Duration::from_secs(2);
const MAX_VOLUME: f32 = 1.5;

const BEST_SOUND: &str = "Best sound (no mic)";
const HEADSET: &str = "Headset (mic on, lower quality)";

pub fn spawn(cmds: Rx, events: Tx) {
    let started = thread::Builder::new().name("telmo-sound".into()).spawn({
        let events = events.clone();
        move || {
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_time()
                .build();
            match runtime {
                Ok(runtime) => runtime.block_on(run(cmds, events)),
                Err(_) => fail_to_start(&events),
            }
        }
    });
    if started.is_err() {
        fail_to_start(&events);
    }
}

fn fail_to_start(events: &Tx) {
    let _ = events.send(Event::Failed(
        "Could not start the audio thread. Restart telmo.".into(),
    ));
}

// What we last read from the server. Commands need the raw volumes and indexes.
#[derive(Default)]
struct Collect {
    pending: u8,
    failed: bool,
    default_sink: Option<String>,
    default_source: Option<String>,
    sinks: Vec<Dev>,
    sources: Vec<Dev>,
    inputs: Vec<Input>,
    cards: Vec<Card>,
}

struct Dev {
    name: String,
    description: String,
    volume: ChannelVolumes,
    muted: bool,
    bluetooth: bool,
    card: Option<u32>,
    index: u32,
    running: bool,
}

struct Input {
    index: u32,
    app: String,
    volume: ChannelVolumes,
    muted: bool,
    sink: u32,
    corked: bool,
}

struct Card {
    index: u32,
    profiles: Vec<CardProfile>,
}

struct CardProfile {
    name: String,
    available: bool,
    active: bool,
}

struct Shared {
    /// Asks the command thread for another refresh.
    wake: UnboundedSender<()>,
    in_flight: bool,
    /// Something changed while a refresh was running.
    stale: bool,
    collect: Collect,
    known: Collect,
}

type State_ = Rc<RefCell<Shared>>;

enum Ended {
    /// The UI is gone; stop the thread.
    Quit,
    /// The server went away; reconnect.
    Lost,
}

async fn run(mut cmds: Rx, events: Tx) {
    let mut announced = false;
    loop {
        match session(&mut cmds, &events).await {
            Ended::Quit => return,
            Ended::Lost => {}
        }
        if !announced {
            announced = true;
            if events.send(Event::Snapshot(Snapshot::default())).is_err() {
                return;
            }
        }
        if !wait_for_retry(&mut cmds, &events).await {
            return;
        }
    }
}

/// Sleeps until the next reconnect attempt, answering commands with a failure.
/// Returns false when the UI is gone.
async fn wait_for_retry(cmds: &mut Rx, events: &Tx) -> bool {
    let retry = tokio::time::sleep(RETRY);
    tokio::pin!(retry);
    loop {
        tokio::select! {
            () = &mut retry => return true,
            cmd = cmds.recv() => {
                if cmd.is_none() {
                    return false;
                }
                let message = "The sound server isn't running. Start it and try again.";
                let _ = events.send(Event::Failed(message.into()));
            }
            () = events.closed() => return false,
        }
    }
}

/// A libpulse mainloop thread and the context living on it.
struct Connection {
    mainloop: Mainloop,
    context: Context,
}

impl Connection {
    /// Starts connecting; `changed` is signalled on every state change.
    fn open(changed: UnboundedSender<()>) -> Option<Self> {
        let mut mainloop = Mainloop::new()?;
        let mut context = Context::new(&mainloop, "telmo")?;
        context.set_state_callback(Some(Box::new(move || {
            let _ = changed.send(());
        })));
        mainloop.lock();
        let started =
            mainloop.start().is_ok() && context.connect(None, FlagSet::NOFLAGS, None).is_ok();
        mainloop.unlock();
        started.then_some(Self { mainloop, context })
    }

    fn locked<R>(&mut self, f: impl FnOnce(&mut Context) -> R) -> R {
        self.mainloop.lock();
        let result = f(&mut self.context);
        self.mainloop.unlock();
        result
    }

    fn state(&mut self) -> State {
        self.locked(|context| context.get_state())
    }
}

impl Drop for Connection {
    fn drop(&mut self) {
        self.locked(|context| {
            context.set_state_callback(None);
            context.set_subscribe_callback(None);
            context.disconnect();
        });
        self.mainloop.stop();
    }
}

async fn session(cmds: &mut Rx, events: &Tx) -> Ended {
    let (changed_tx, mut changed) = unbounded_channel();
    let Some(mut conn) = Connection::open(changed_tx) else {
        return Ended::Lost;
    };
    loop {
        match conn.state() {
            State::Ready => break,
            State::Failed | State::Terminated => return Ended::Lost,
            _ => {}
        }
        tokio::select! {
            _ = changed.recv() => {}
            () = events.closed() => return Ended::Quit,
        }
    }

    let (wake_tx, mut wake) = unbounded_channel();
    let shared: State_ = Rc::new(RefCell::new(Shared {
        wake: wake_tx.clone(),
        in_flight: false,
        stale: false,
        collect: Collect::default(),
        known: Collect::default(),
    }));
    let mask = InterestMaskSet::SINK
        | InterestMaskSet::SOURCE
        | InterestMaskSet::SINK_INPUT
        | InterestMaskSet::CARD
        | InterestMaskSet::SERVER;
    conn.locked(|context| {
        context.subscribe(mask, |_| {});
        context.set_subscribe_callback(Some(Box::new(move |_, _, _| {
            let _ = wake_tx.send(());
        })));
    });
    // Read everything once right away.
    let _ = shared.borrow().wake.send(());

    loop {
        tokio::select! {
            cmd = cmds.recv() => match cmd {
                Some(cmd) => conn.locked(|context| apply(context, &shared, events, cmd)),
                None => return Ended::Quit,
            },
            _ = wake.recv() => {
                // Let a burst of changes settle, then read once.
                tokio::time::sleep(DEBOUNCE).await;
                while wake.try_recv().is_ok() {}
                conn.locked(|context| request_refresh(context, &shared, events));
            }
            _ = changed.recv() => {
                if conn.state() != State::Ready {
                    return Ended::Lost;
                }
            }
            () = events.closed() => return Ended::Quit,
        }
    }
}

fn request_refresh(context: &Context, shared: &State_, events: &Tx) {
    if shared.borrow().in_flight {
        shared.borrow_mut().stale = true;
    } else {
        refresh(context, shared, events);
    }
}

fn refresh(context: &Context, shared: &State_, events: &Tx) {
    {
        let mut s = shared.borrow_mut();
        s.in_flight = true;
        s.collect = Collect {
            pending: 5,
            ..Collect::default()
        };
    }
    let intro = context.introspect();

    intro.get_server_info({
        let (shared, events) = (shared.clone(), events.clone());
        move |info| {
            {
                let mut s = shared.borrow_mut();
                s.collect.default_sink = info.default_sink_name.as_deref().map(str::to_owned);
                s.collect.default_source = info.default_source_name.as_deref().map(str::to_owned);
            }
            step(&shared, &events);
        }
    });
    intro.get_sink_info_list({
        let (shared, events) = (shared.clone(), events.clone());
        move |item| {
            if let ListResult::Item(info) = &item {
                let dev = sink_dev(info);
                shared.borrow_mut().collect.sinks.extend(dev);
            }
            list_step(item, &shared, &events);
        }
    });
    intro.get_source_info_list({
        let (shared, events) = (shared.clone(), events.clone());
        move |item| {
            if let ListResult::Item(info) = &item
                && info.monitor_of_sink.is_none()
            {
                let dev = source_dev(info);
                shared.borrow_mut().collect.sources.extend(dev);
            }
            list_step(item, &shared, &events);
        }
    });
    intro.get_sink_input_info_list({
        let (shared, events) = (shared.clone(), events.clone());
        move |item| {
            if let ListResult::Item(info) = &item {
                shared.borrow_mut().collect.inputs.push(input(info));
            }
            list_step(item, &shared, &events);
        }
    });
    intro.get_card_info_list({
        let (shared, events) = (shared.clone(), events.clone());
        move |item| {
            if let ListResult::Item(info) = &item {
                shared.borrow_mut().collect.cards.push(card(info));
            }
            list_step(item, &shared, &events);
        }
    });
}

fn list_step<T: ?Sized>(item: ListResult<&T>, shared: &State_, events: &Tx) {
    match item {
        ListResult::Item(_) => {}
        ListResult::End => step(shared, events),
        ListResult::Error => {
            shared.borrow_mut().collect.failed = true;
            step(shared, events);
        }
    }
}

/// One of the five queries finished; publish when all have.
fn step(shared: &State_, events: &Tx) {
    let mut s = shared.borrow_mut();
    s.collect.pending = s.collect.pending.saturating_sub(1);
    if s.collect.pending > 0 {
        return;
    }
    s.in_flight = false;
    if std::mem::take(&mut s.stale) {
        let _ = s.wake.send(());
    }
    if s.collect.failed {
        let message = "Could not read the audio devices. Check that the sound server is running.";
        let _ = events.send(Event::Failed(message.into()));
        return;
    }
    s.known = std::mem::take(&mut s.collect);
    let _ = events.send(Event::Snapshot(snapshot(&s.known)));
}

fn is_bluetooth(props: &Proplist) -> bool {
    props.get_str("device.bus").as_deref() == Some("bluetooth")
        || props.get_str("device.api").as_deref() == Some("bluez5")
}

fn sink_dev(info: &SinkInfo) -> Option<Dev> {
    Some(Dev {
        name: info.name.as_deref()?.to_owned(),
        description: info.description.as_deref().unwrap_or_default().to_owned(),
        volume: info.volume,
        muted: info.mute,
        bluetooth: is_bluetooth(&info.proplist),
        card: info.card,
        index: info.index,
        running: info.state == libpulse_binding::def::SinkState::Running,
    })
}

fn source_dev(info: &SourceInfo) -> Option<Dev> {
    Some(Dev {
        name: info.name.as_deref()?.to_owned(),
        description: info.description.as_deref().unwrap_or_default().to_owned(),
        volume: info.volume,
        muted: info.mute,
        bluetooth: is_bluetooth(&info.proplist),
        card: info.card,
        index: info.index,
        running: info.state == libpulse_binding::def::SourceState::Running,
    })
}

fn input(info: &SinkInputInfo) -> Input {
    let props = &info.proplist;
    let app = ["application.name", "application.process.binary"]
        .iter()
        .find_map(|key| props.get_str(key))
        .or_else(|| info.name.as_deref().map(str::to_owned))
        .unwrap_or_else(|| "Unknown app".into());
    Input {
        index: info.index,
        app,
        volume: info.volume,
        muted: info.mute,
        sink: info.sink,
        corked: info.corked,
    }
}

fn card(info: &CardInfo) -> Card {
    let active = info
        .active_profile
        .as_ref()
        .and_then(|p| p.name.as_deref().map(str::to_owned));
    let profiles = info
        .profiles
        .iter()
        .filter_map(|p| {
            let name = p.name.as_deref()?.to_owned();
            Some(CardProfile {
                available: p.available,
                active: active.as_deref() == Some(name.as_str()),
                name,
            })
        })
        .collect();
    Card {
        index: info.index,
        profiles,
    }
}

fn snapshot(c: &Collect) -> Snapshot {
    let devices = |list: &[Dev], default: &Option<String>| {
        list.iter()
            .map(|d| device(d, default.as_deref() == Some(d.name.as_str()), &c.cards))
            .collect()
    };
    Snapshot {
        outputs: devices(&c.sinks, &c.default_sink),
        inputs: devices(&c.sources, &c.default_source),
        streams: c.inputs.iter().map(|i| stream(i, &c.sinks)).collect(),
        caps: Caps {
            per_app: true,
            profiles: true,
        },
    }
}

fn level(volume: &ChannelVolumes) -> f32 {
    volume.avg().0 as f32 / Volume::NORMAL.0 as f32
}

fn device(d: &Dev, default: bool, cards: &[Card]) -> Device {
    let profiles = match d
        .card
        .and_then(|index| cards.iter().find(|c| c.index == index))
    {
        Some(card) if d.bluetooth => bluetooth_profiles(card),
        _ => Vec::new(),
    };
    Device {
        id: d.name.clone(),
        name: if d.description.is_empty() {
            d.name.clone()
        } else {
            d.description.clone()
        },
        default,
        volume: Some(level(&d.volume)),
        muted: d.muted,
        bluetooth: d.bluetooth,
        profiles,
        playing: d.running,
        eq: None,
    }
}

/// Collapses the card's profiles into the two modes people care about. The id
/// is the active profile of that kind, else the first available one.
fn bluetooth_profiles(card: &Card) -> Vec<Profile> {
    let kinds = [("a2dp", BEST_SOUND), ("headset-head-unit", HEADSET)];
    kinds
        .iter()
        .filter_map(|(prefix, label)| {
            let matching: Vec<&CardProfile> = card
                .profiles
                .iter()
                .filter(|p| p.name != "off" && p.name.starts_with(prefix))
                .filter(|p| p.available || p.active)
                .collect();
            let chosen = matching.iter().find(|p| p.active).or(matching.first())?;
            Some(Profile {
                id: chosen.name.clone(),
                name: (*label).into(),
                active: chosen.active,
            })
        })
        .collect()
}

fn stream(i: &Input, sinks: &[Dev]) -> Stream {
    Stream {
        id: i.index.to_string(),
        app: i.app.clone(),
        volume: level(&i.volume),
        muted: i.muted,
        device: sinks
            .iter()
            .find(|s| s.index == i.sink)
            .map(|s| s.name.clone()),
        playing: !i.corked,
    }
}

// Commands

type Done = Option<Box<dyn FnMut(bool) + 'static>>;

/// Callback that turns a refused change into a toast.
fn reply(events: &Tx, what: &str) -> Done {
    let events = events.clone();
    let message = format!("The sound server refused to {what}. Try again.");
    Some(Box::new(move |ok| {
        if !ok {
            let _ = events.send(Event::Failed(message.clone()));
        }
    }))
}

fn gone(events: &Tx, what: &str) {
    let message = format!("{what} is no longer available. Pick another one.");
    let _ = events.send(Event::Failed(message));
}

fn volume_of(level: f32) -> Volume {
    let level = if level.is_finite() { level } else { 0.0 };
    Volume((level.clamp(0.0, MAX_VOLUME) * Volume::NORMAL.0 as f32).round() as u32)
}

fn devices(c: &Collect, direction: Direction) -> &[Dev] {
    match direction {
        Direction::Output => &c.sinks,
        Direction::Input => &c.sources,
    }
}

fn apply(context: &mut Context, shared: &State_, events: &Tx, cmd: Cmd) {
    let shared = shared.borrow();
    let known = &shared.known;
    let mut intro = context.introspect();
    match cmd {
        Cmd::SetDefault(direction, id) => {
            if !devices(known, direction).iter().any(|d| d.name == id) {
                return gone(events, "That device");
            }
            let events = events.clone();
            let done = move |ok: bool| {
                if !ok {
                    let message = "The sound server refused to switch devices. Try again.";
                    let _ = events.send(Event::Failed(message.into()));
                }
            };
            match direction {
                Direction::Output => context.set_default_sink(&id, done),
                Direction::Input => context.set_default_source(&id, done),
            };
        }
        Cmd::SetVolume(Target::Device(direction, id), level) => {
            let Some(d) = devices(known, direction).iter().find(|d| d.name == id) else {
                return gone(events, "That device");
            };
            let mut volume = d.volume;
            volume.scale(volume_of(level));
            let done = reply(events, "change the volume");
            match direction {
                Direction::Output => intro.set_sink_volume_by_name(&id, &volume, done),
                Direction::Input => intro.set_source_volume_by_name(&id, &volume, done),
            };
        }
        Cmd::SetVolume(Target::Stream(id), level) => {
            let Some(i) = known.inputs.iter().find(|i| i.index.to_string() == id) else {
                return gone(events, "That app");
            };
            let mut volume = i.volume;
            volume.scale(volume_of(level));
            intro.set_sink_input_volume(i.index, &volume, reply(events, "change the volume"));
        }
        Cmd::SetMute(Target::Device(direction, id), muted) => {
            if !devices(known, direction).iter().any(|d| d.name == id) {
                return gone(events, "That device");
            }
            let done = reply(events, "change the mute setting");
            match direction {
                Direction::Output => intro.set_sink_mute_by_name(&id, muted, done),
                Direction::Input => intro.set_source_mute_by_name(&id, muted, done),
            };
        }
        Cmd::SetMute(Target::Stream(id), muted) => {
            let Some(i) = known.inputs.iter().find(|i| i.index.to_string() == id) else {
                return gone(events, "That app");
            };
            intro.set_sink_input_mute(i.index, muted, reply(events, "change the mute setting"));
        }
        Cmd::MoveStream { stream, device } => {
            let Some(i) = known.inputs.iter().find(|i| i.index.to_string() == stream) else {
                return gone(events, "That app");
            };
            if !known.sinks.iter().any(|d| d.name == device) {
                return gone(events, "That output");
            }
            intro.move_sink_input_by_name(i.index, &device, reply(events, "move that app"));
        }
        Cmd::SetProfile { device, profile } => {
            let card = known
                .sinks
                .iter()
                .chain(&known.sources)
                .find(|d| d.name == device)
                .and_then(|d| d.card);
            let Some(card) = card else {
                return gone(events, "That device");
            };
            intro.set_card_profile_by_index(card, &profile, reply(events, "change the mode"));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::sync::mpsc::unbounded_channel;

    async fn next_snapshot(events: &mut tokio::sync::mpsc::UnboundedReceiver<Event>) -> Snapshot {
        let wait = async {
            match events.recv().await {
                Some(Event::Snapshot(s)) => s,
                Some(Event::Failed(m)) => panic!("failed: {m}"),
                Some(_) => panic!("the backend only sends spectra to the visualizer"),
                None => panic!("backend stopped"),
            }
        };
        tokio::time::timeout(Duration::from_secs(5), wait)
            .await
            .expect("no snapshot in 5 s")
    }

    /// Touches the real sound server and restores what it changed.
    #[tokio::test(flavor = "current_thread")]
    #[ignore = "needs a running PulseAudio-compatible server"]
    async fn volume_mute_default_round_trip() {
        let (cmd_tx, cmd_rx) = unbounded_channel();
        let (event_tx, mut event_rx) = unbounded_channel();
        spawn(cmd_rx, event_tx);
        let before = next_snapshot(&mut event_rx).await;
        let out = before.outputs.iter().find(|d| d.default).expect("default");
        let target = Target::Device(Direction::Output, out.id.clone());

        cmd_tx.send(Cmd::SetVolume(target.clone(), 0.37)).unwrap();
        cmd_tx
            .send(Cmd::SetMute(target.clone(), !out.muted))
            .unwrap();
        let mut after = next_snapshot(&mut event_rx).await;
        while after
            .outputs
            .iter()
            .find(|d| d.id == out.id)
            .map(|d| d.muted)
            == Some(out.muted)
        {
            after = next_snapshot(&mut event_rx).await;
        }
        let now = after.outputs.iter().find(|d| d.id == out.id).unwrap();
        assert!(
            (now.volume.unwrap() - 0.37).abs() < 0.01,
            "{:?}",
            now.volume
        );

        if let Some(other) = before.outputs.iter().find(|d| !d.default) {
            cmd_tx
                .send(Cmd::SetDefault(Direction::Output, other.id.clone()))
                .unwrap();
            let mut s = next_snapshot(&mut event_rx).await;
            while !s.outputs.iter().any(|d| d.id == other.id && d.default) {
                s = next_snapshot(&mut event_rx).await;
            }
        }

        cmd_tx
            .send(Cmd::SetDefault(Direction::Output, out.id.clone()))
            .unwrap();
        cmd_tx
            .send(Cmd::SetVolume(target.clone(), out.volume.unwrap()))
            .unwrap();
        cmd_tx.send(Cmd::SetMute(target, out.muted)).unwrap();
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
}
