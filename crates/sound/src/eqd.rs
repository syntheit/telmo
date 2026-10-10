//! The Linux EQ engine: `telmo-sound eq-daemon`.
//!
//! It reads the same `eq.json` the popup writes and, for every output that is
//! present and has a non-flat entry, runs a PipeWire filter-chain in front of
//! it. Audio reaches the chain through a WirePlumber *smart filter*: the
//! chain's virtual sink (`telmo_eq.in.<sink>`) is marked `filter.smart` with
//! the real sink as its `filter.smart.target`, so WirePlumber sends every
//! stream bound for the real sink (including "the default sink") through the
//! chain and on to the real sink. The default output never changes, so
//! volume keys, `wpctl` and the popup keep controlling the real sink.
//!
//! Each chain runs in its own `pipewire -c <generated conf>` child process,
//! the way PipeWire's own filter-chain docs run one. The process is a client
//! of the running server (no socket of its own), which makes teardown exact
//! (kill the child: the chain and its nodes vanish and WirePlumber re-links
//! the streams to the real sink), keeps a bad graph from taking down the
//! daemon, and needs no `pw-cli` session kept open (modules loaded by
//! `pw-cli` die with it). A change of the entry restarts that chain, a gap of
//! a fraction of a second.
//!
//! The pure parts (which outputs get a chain, the generated config) are
//! plain functions with tests; only `run` touches the system.

#![cfg_attr(not(target_os = "linux"), allow(dead_code))]

use crate::eq::{Config, Entry, FILTER_IN, FILTER_OUT, Filter, Shape};
use crate::model::Device;
use serde_json::{Value, json};
use std::collections::BTreeMap;

/// One chain to run: the real sink it sits in front of and what it applies.
#[derive(Debug, Clone, PartialEq)]
pub struct Spec {
    /// The real sink's PipeWire `node.name`.
    pub node: String,
    pub description: String,
    pub preamp: f64,
    pub filters: Vec<Filter>,
}

fn is_flat(entry: &Entry) -> bool {
    entry.preamp == 0.0 && entry.filters.is_empty()
}

/// The chains that should run: one per present output whose entry changes the
/// sound. Everything else (disabled, flat, no entry, an output that is gone)
/// gets none, which is how tearing down works.
pub fn plan(config: &Config, outputs: &[Device]) -> BTreeMap<String, Spec> {
    if !config.enabled {
        return BTreeMap::new();
    }
    outputs
        .iter()
        .filter_map(|device| {
            let target = device.eq.as_ref()?;
            let entry = config.devices.get(&target.key)?;
            if is_flat(entry) {
                return None;
            }
            Some((
                target.key.clone(),
                Spec {
                    node: target.key.clone(),
                    description: device.name.clone(),
                    preamp: entry.preamp,
                    filters: entry.filters.clone(),
                },
            ))
        })
        .collect()
}

fn label(shape: Shape) -> &'static str {
    match shape {
        Shape::Peak => "bq_peaking",
        Shape::LowShelf => "bq_lowshelf",
        Shape::HighShelf => "bq_highshelf",
        Shape::HighPass => "bq_highpass",
    }
}

fn filter_graph(spec: &Spec) -> Value {
    let mut nodes = Vec::new();
    if spec.preamp != 0.0 {
        // A 0 Hz high shelf is a flat gain (PipeWire's own idiom for a preamp).
        nodes.push(json!({
            "type": "builtin", "name": "preamp", "label": "bq_highshelf",
            "control": {"Freq": 0.0, "Q": 1.0, "Gain": spec.preamp},
        }));
    }
    for (i, f) in spec.filters.iter().enumerate() {
        let control = if f.shape == Shape::HighPass {
            json!({"Freq": f.freq, "Q": f.q})
        } else {
            json!({"Freq": f.freq, "Q": f.q, "Gain": f.gain})
        };
        nodes.push(json!({
            "type": "builtin", "name": format!("f{i}"), "label": label(f.shape),
            "control": control,
        }));
    }
    let names: Vec<&str> = nodes.iter().filter_map(|n| n["name"].as_str()).collect();
    let links: Vec<Value> = names
        .windows(2)
        .map(|pair| json!({"output": format!("{}:Out", pair[0]), "input": format!("{}:In", pair[1])}))
        .collect();
    json!({"nodes": nodes, "links": links})
}

/// The config file for the `pipewire -c` child that runs one chain. It is
/// JSON, which PipeWire's config parser accepts, with the same base modules
/// as PipeWire's shipped `filter-chain.conf`.
pub fn chain_conf(spec: &Spec) -> String {
    let group = format!("telmo-eq-{}", spec.node);
    let conf = json!({
        "context.properties": {"log.level": 1},
        "context.spa-libs": {
            "audio.convert.*": "audioconvert/libspa-audioconvert",
            "support.*": "support/libspa-support",
        },
        "context.modules": [
            {"name": "libpipewire-module-rt", "flags": ["ifexists", "nofail"]},
            {"name": "libpipewire-module-protocol-native"},
            {"name": "libpipewire-module-client-node"},
            {"name": "libpipewire-module-adapter"},
            {"name": "libpipewire-module-filter-chain", "args": {
                "node.description": format!("EQ for {}", spec.description),
                "media.name": format!("EQ for {}", spec.description),
                "filter.graph": filter_graph(spec),
                "audio.channels": 2,
                "audio.position": ["FL", "FR"],
                "capture.props": {
                    "node.name": format!("{FILTER_IN}{}", spec.node),
                    "node.link-group": group,
                    "media.class": "Audio/Sink",
                    "filter.smart": true,
                    "filter.smart.name": group,
                    "filter.smart.target": {"node.name": spec.node},
                },
                "playback.props": {
                    "node.name": format!("{FILTER_OUT}{}", spec.node),
                    "node.link-group": group,
                    "node.passive": true,
                    "stream.dont-remix": true,
                },
            }},
        ],
    });
    serde_json::to_string_pretty(&conf).unwrap_or_default() + "\n"
}

/// File-name-safe form of a node name.
fn file_stem(node: &str) -> String {
    node.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.') {
                c
            } else {
                '_'
            }
        })
        .collect()
}

/// The names in a buffer of inotify events (`struct inotify_event`: wd, mask,
/// cookie, len, then `len` bytes of NUL-padded name).
fn event_names(buf: &[u8]) -> Vec<String> {
    const HEADER: usize = 16;
    let mut names = Vec::new();
    let mut at = 0;
    while at + HEADER <= buf.len() {
        let len = u32::from_ne_bytes([buf[at + 12], buf[at + 13], buf[at + 14], buf[at + 15]]);
        let end = (at + HEADER + len as usize).min(buf.len());
        let raw = &buf[at + HEADER..end];
        let raw = raw.split(|b| *b == 0).next().unwrap_or_default();
        names.push(String::from_utf8_lossy(raw).into_owned());
        at = end;
    }
    names
}

#[cfg(target_os = "linux")]
pub use runtime::run;

#[cfg(target_os = "linux")]
mod runtime {
    use super::*;
    use crate::backend::{self, Event};
    use std::{
        ffi::CString,
        path::{Path, PathBuf},
        process::{Child, Command, Stdio},
        time::Duration,
    };
    use tokio::sync::mpsc::{UnboundedSender, unbounded_channel};

    const TICK: Duration = Duration::from_secs(5);
    const SETTLE: Duration = Duration::from_millis(150);

    struct Running {
        spec: Spec,
        child: Child,
    }

    impl Drop for Running {
        fn drop(&mut self) {
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
    }

    pub async fn run() {
        let Some(dir) = telmo_kit::state::dir() else {
            return crate::fail("no home directory");
        };
        let (file_tx, mut file_rx) = unbounded_channel();
        watch(&dir, file_tx);
        // Keeping `_cmds` alive keeps the backend running.
        let (_cmds, cmd_rx) = unbounded_channel();
        let (event_tx, mut event_rx) = unbounded_channel();
        backend::spawn(false, cmd_rx, event_tx);

        let mut outputs: Vec<Device> = Vec::new();
        let mut running: BTreeMap<String, Running> = BTreeMap::new();
        let mut tick = tokio::time::interval(TICK);
        loop {
            tokio::select! {
                event = event_rx.recv() => match event {
                    Some(Event::Snapshot(snapshot)) => {
                        outputs = snapshot.outputs;
                        seed(&outputs);
                    }
                    Some(_) => continue,
                    None => return,
                },
                Some(()) = file_rx.recv() => {
                    // A save is a burst of events.
                    tokio::time::sleep(SETTLE).await;
                    while file_rx.try_recv().is_ok() {}
                }
                _ = tick.tick() => {}
            }
            reconcile(&Config::load(), &outputs, &mut running);
        }
    }

    /// Gives outputs without an entry their default, like Telmo.app does.
    fn seed(outputs: &[Device]) {
        let mut probe = Config::load();
        if probe.seed(outputs)
            && let Err(e) = Config::update(|config| {
                config.seed(outputs);
            })
        {
            eprintln!("telmo-sound eq-daemon: could not save the EQ settings: {e}");
        }
    }

    fn reconcile(config: &Config, outputs: &[Device], running: &mut BTreeMap<String, Running>) {
        let wanted = plan(config, outputs);
        // Dead chains, chains nobody wants and chains whose settings changed.
        running.retain(|key, r| {
            let alive = matches!(r.child.try_wait(), Ok(None));
            alive && wanted.get(key) == Some(&r.spec)
        });
        for (key, spec) in wanted {
            if running.contains_key(&key) {
                continue;
            }
            match start(&spec) {
                Ok(child) => {
                    running.insert(key, Running { spec, child });
                }
                Err(e) => eprintln!("telmo-sound eq-daemon: could not start the EQ for {key}: {e}"),
            }
        }
    }

    fn start(spec: &Spec) -> std::io::Result<Child> {
        let base = std::env::var_os("XDG_RUNTIME_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(std::env::temp_dir)
            .join("telmo-eq");
        std::fs::create_dir_all(&base)?;
        let conf = base.join(format!("{}.conf", file_stem(&spec.node)));
        std::fs::write(&conf, chain_conf(spec))?;
        let pipewire = std::env::var_os("TELMO_PIPEWIRE").unwrap_or_else(|| "pipewire".into());
        Command::new(pipewire)
            .arg("-c")
            .arg(&conf)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .spawn()
    }

    /// Signals `tx` whenever `eq.json` in `dir` is written, renamed into
    /// place or removed (inotify on the directory, since saves replace the
    /// file). Falls back to a slow poll if inotify is unavailable.
    fn watch(dir: &Path, tx: UnboundedSender<()>) {
        let _ = std::fs::create_dir_all(dir);
        let fd = CString::new(dir.as_os_str().as_encoded_bytes())
            .ok()
            .and_then(|path| {
                // SAFETY: plain syscalls with a valid NUL-terminated path.
                unsafe {
                    let fd = libc::inotify_init1(libc::IN_CLOEXEC);
                    let mask = libc::IN_CLOSE_WRITE
                        | libc::IN_MOVED_TO
                        | libc::IN_MOVED_FROM
                        | libc::IN_DELETE;
                    (fd >= 0 && libc::inotify_add_watch(fd, path.as_ptr(), mask) >= 0).then_some(fd)
                }
            });
        std::thread::spawn(move || {
            let Some(fd) = fd else {
                eprintln!("telmo-sound eq-daemon: no inotify; checking eq.json every 2 s");
                while tx.send(()).is_ok() {
                    std::thread::sleep(Duration::from_secs(2));
                }
                return;
            };
            let mut buf = [0u8; 4096];
            loop {
                // SAFETY: `buf` is valid for `buf.len()` bytes.
                let n = unsafe { libc::read(fd, buf.as_mut_ptr().cast(), buf.len()) };
                if n < 0
                    && std::io::Error::last_os_error().kind() == std::io::ErrorKind::Interrupted
                {
                    continue;
                }
                let Ok(n) = usize::try_from(n) else { return };
                if event_names(&buf[..n]).iter().any(|name| name == "eq.json")
                    && tx.send(()).is_err()
                {
                    return;
                }
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::eq::Config;

    fn sink(name: &str, node: &str) -> Device {
        Device {
            id: node.into(),
            name: name.into(),
            default: false,
            volume: Some(0.5),
            muted: false,
            bluetooth: false,
            profiles: Vec::new(),
            playing: false,
            eq: crate::eq::sink_target(node),
        }
    }

    fn jack() -> Device {
        sink(
            "Starship/Matisse HD Audio Controller Analog Stereo",
            "alsa_output.pci-0000_28_00.4.analog-stereo",
        )
    }

    fn mac() -> Device {
        sink("Mac mini speakers", "mac-speakers")
    }

    fn hdmi() -> Device {
        sink("HDMI", "alsa_output.pci-0000_26_00.1.hdmi-stereo")
    }

    #[test]
    fn only_non_flat_present_outputs_get_a_chain() {
        let outputs = [jack(), mac(), hdmi()];
        let mut config = Config::default();
        config.seed(&outputs);
        let wanted = plan(&config, &outputs);
        // The jack's "+ Sub-bass" and the Mac mini's tune; HDMI stays flat.
        assert_eq!(
            wanted.keys().collect::<Vec<_>>(),
            ["alsa_output.pci-0000_28_00.4.analog-stereo", "mac-speakers"]
        );
        assert_eq!(wanted["mac-speakers"].preamp, -3.0);
        // An unplugged output has no chain.
        assert_eq!(plan(&config, &[mac()]).len(), 1);
        assert!(plan(&config, &[]).is_empty());
    }

    #[test]
    fn off_and_flat_tear_everything_down() {
        let outputs = [jack(), mac()];
        let mut config = Config::default();
        config.seed(&outputs);
        config.enabled = false;
        assert!(plan(&config, &outputs).is_empty());
        config.enabled = true;
        config.set(&mac(), 1, 0); // "Flat"
        assert_eq!(plan(&config, &outputs).len(), 1);
        config.set(&mac(), 1, -2); // flat plus a bass cut is not flat
        assert_eq!(plan(&config, &outputs).len(), 2);
    }

    #[test]
    fn outputs_without_a_target_are_left_alone() {
        let mut device = mac();
        device.eq = None;
        let mut config = Config::default();
        config.devices.insert(
            "mac-speakers".into(),
            Entry {
                name: "x".into(),
                preset: "x".into(),
                bass: 0,
                preamp: -3.0,
                filters: Vec::new(),
            },
        );
        assert!(plan(&config, &[device]).is_empty());
    }

    #[test]
    fn the_mac_mini_chain_config() {
        let mut config = Config::default();
        config.seed(&[mac()]);
        let spec = &plan(&config, &[mac()])["mac-speakers"];
        insta::assert_snapshot!(chain_conf(spec));
    }

    #[test]
    fn every_filter_shape_has_its_label_and_the_chain_is_linked() {
        let spec = Spec {
            node: "n".into(),
            description: "n".into(),
            preamp: 0.0,
            filters: vec![
                Filter {
                    shape: Shape::LowShelf,
                    freq: 100.0,
                    gain: 2.0,
                    q: 0.7,
                },
                Filter {
                    shape: Shape::Peak,
                    freq: 1000.0,
                    gain: -1.0,
                    q: 1.0,
                },
                Filter {
                    shape: Shape::HighShelf,
                    freq: 8000.0,
                    gain: 1.5,
                    q: 0.7,
                },
                Filter {
                    shape: Shape::HighPass,
                    freq: 40.0,
                    gain: 0.0,
                    q: 0.7,
                },
            ],
        };
        let graph = filter_graph(&spec);
        let labels: Vec<&str> = graph["nodes"]
            .as_array()
            .unwrap()
            .iter()
            .map(|n| n["label"].as_str().unwrap())
            .collect();
        assert_eq!(
            labels,
            ["bq_lowshelf", "bq_peaking", "bq_highshelf", "bq_highpass"]
        );
        assert_eq!(graph["links"].as_array().unwrap().len(), 3);
        // A high-pass has no gain; no preamp node when the preamp is 0.
        assert!(graph["nodes"][3]["control"].get("Gain").is_none());
        let single = Spec {
            filters: spec.filters[..1].to_vec(),
            ..spec
        };
        assert!(
            filter_graph(&single)["links"]
                .as_array()
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn inotify_events_yield_their_names() {
        let mut buf = Vec::new();
        for name in ["eq.json", "eq.lock"] {
            buf.extend_from_slice(&[0; 12]);
            buf.extend_from_slice(&16u32.to_ne_bytes());
            let mut padded = name.as_bytes().to_vec();
            padded.resize(16, 0);
            buf.extend_from_slice(&padded);
        }
        assert_eq!(event_names(&buf), ["eq.json", "eq.lock"]);
        assert!(event_names(&buf[..10]).is_empty());
    }

    #[test]
    fn file_names_are_safe() {
        assert_eq!(file_stem("bluez_output.AA:BB/1"), "bluez_output.AA_BB_1");
    }
}
