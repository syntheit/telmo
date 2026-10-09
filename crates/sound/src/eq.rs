//! The built-in EQ presets and the file Telmo.app reads them from.
//!
//! This module owns every choice: which presets an output offers, which one
//! is on, the bass nudge. Telmo.app only runs the audio, so each entry in
//! `eq.json` carries the finished filters, ready to apply:
//!
//! ```json
//! {"enabled": true, "devices": {"<key>": {"name": "MacBook Air Speakers",
//!   "preset": "Speakers +", "bass": 0, "preamp": -3.0,
//!   "filters": [{"type": "peak", "freq": 160, "gain": 3, "q": 0.7}]}}}
//! ```
//!
//! The key is the device UID, plus the data source on built-in devices. An
//! output without an entry is left alone, so `seed` gives every output its
//! kind's default (the popup does it whenever it sees a device, and
//! `telmo-sound eq-seed` does it for Telmo.app).

use crate::model::{Device, EqTarget};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, io};

const STATE: &str = "eq";
/// The bass nudge is a low shelf here, this far up.
const NUDGE_FREQ: f64 = 110.0;
pub const NUDGE_LIMIT: i32 = 6;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Shape {
    Peak,
    LowShelf,
    HighShelf,
    HighPass,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Filter {
    #[serde(rename = "type")]
    pub shape: Shape,
    pub freq: f64,
    /// Ignored by a high-pass.
    pub gain: f64,
    pub q: f64,
}

const fn peak(freq: f64, gain: f64, q: f64) -> Filter {
    Filter {
        shape: Shape::Peak,
        freq,
        gain,
        q,
    }
}

const fn low_shelf(freq: f64, gain: f64, q: f64) -> Filter {
    Filter {
        shape: Shape::LowShelf,
        freq,
        gain,
        q,
    }
}

const fn high_shelf(freq: f64, gain: f64, q: f64) -> Filter {
    Filter {
        shape: Shape::HighShelf,
        freq,
        gain,
        q,
    }
}

const fn high_pass(freq: f64, q: f64) -> Filter {
    Filter {
        shape: Shape::HighPass,
        freq,
        gain: 0.0,
        q,
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Preset {
    pub name: &'static str,
    /// Negative: makes room for the boosts so nothing clips.
    pub preamp: f64,
    pub filters: &'static [Filter],
    /// A warning shown under the curve.
    pub note: Option<&'static str>,
}

const fn preset(name: &'static str, preamp: f64, filters: &'static [Filter]) -> Preset {
    Preset {
        name,
        preamp,
        filters,
        note: None,
    }
}

const FLAT: Preset = preset("Flat", 0.0, &[]);
const BASS_BOOST: Preset = preset(
    "Bass boost",
    -4.0,
    &[low_shelf(90.0, 4.0, 0.7), peak(200.0, 1.0, 1.0)],
);
const VOCAL: Preset = preset(
    "Vocal",
    -2.5,
    &[
        peak(250.0, -2.0, 1.0),
        peak(3000.0, 2.5, 1.0),
        high_shelf(8000.0, 1.5, 0.7),
    ],
);
const LATE_NIGHT: Preset = preset(
    "Late night",
    -3.0,
    &[
        low_shelf(120.0, -3.0, 0.7),
        peak(3000.0, 1.5, 0.8),
        high_shelf(9000.0, -2.0, 0.7),
    ],
);

const SPEAKERS: [Preset; 6] = [
    preset("Speakers +", -3.0, &[peak(160.0, 3.0, 0.7)]),
    preset(
        "Speakers ++",
        -4.5,
        &[peak(170.0, 4.5, 0.7), high_pass(40.0, 0.7)],
    ),
    FLAT,
    BASS_BOOST,
    VOCAL,
    LATE_NIGHT,
];

/// The earbuds' own app EQ (+8 / +7 / +5 at 31.5, 63 and 125 Hz) copied
/// here, so it works for every app on the Mac. Their EQ must then be flat.
const EARFUN: [Preset; 5] = [
    FLAT,
    Preset {
        name: "Your EarFun EQ",
        preamp: -10.0,
        filters: &[
            peak(31.5, 8.0, 1.4),
            peak(63.0, 7.0, 1.4),
            peak(125.0, 5.0, 1.4),
        ],
        note: Some("Set the EarFun app's EQ to flat while this is on."),
    },
    BASS_BOOST,
    VOCAL,
    LATE_NIGHT,
];

/// The headphone jack, tuned for the TRUTHEAR x Crinacle Zero:RED with its
/// Bass+ adapter, which already has plenty of bass.
const WIRED: [Preset; 6] = [
    preset("As is", 0.0, &[]),
    preset("+ Sub-bass", -2.0, &[low_shelf(100.0, 2.0, 0.7)]),
    preset("+ Sub-bass ++", -3.0, &[low_shelf(100.0, 3.0, 0.7)]),
    preset("Sub focus", -2.5, &[peak(50.0, 2.5, 0.8)]),
    preset(
        "Warm",
        -3.0,
        &[low_shelf(150.0, 2.5, 0.7), peak(3500.0, -1.0, 1.0)],
    ),
    LATE_NIGHT,
];

const GENERIC: [Preset; 4] = [FLAT, BASS_BOOST, VOCAL, LATE_NIGHT];

/// Which list of presets an output gets.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Speakers,
    EarFun,
    Wired,
    Generic,
}

impl Kind {
    pub fn of(name: &str, target: &EqTarget) -> Kind {
        if target.builtin {
            if target.key.ends_with("#hdpn") || name.contains("Headphones") {
                Kind::Wired
            } else {
                Kind::Speakers
            }
        } else if name.contains("EarFun") {
            Kind::EarFun
        } else {
            Kind::Generic
        }
    }

    pub fn presets(self) -> &'static [Preset] {
        match self {
            Kind::Speakers => &SPEAKERS,
            Kind::EarFun => &EARFUN,
            Kind::Wired => &WIRED,
            Kind::Generic => &GENERIC,
        }
    }

    /// Index of the preset an output starts with. The earbuds keep their own
    /// EQ, so they start flat.
    pub fn default_preset(self) -> usize {
        match self {
            Kind::Wired => 1,
            _ => 0,
        }
    }
}

/// A preset with the bass nudge folded in: what the audio engine applies.
#[derive(Debug, Clone, PartialEq)]
pub struct Resolved {
    pub preamp: f64,
    pub filters: Vec<Filter>,
}

/// The nudge is a low shelf; boosting it lowers the preamp by the same
/// amount, so it can't clip.
pub fn resolve(preset: &Preset, bass: i32) -> Resolved {
    let mut filters = preset.filters.to_vec();
    if bass != 0 {
        filters.push(low_shelf(NUDGE_FREQ, f64::from(bass), 0.7));
    }
    Resolved {
        preamp: preset.preamp - f64::from(bass.max(0)),
        filters,
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Entry {
    pub name: String,
    pub preset: String,
    pub bass: i32,
    pub preamp: f64,
    pub filters: Vec<Filter>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    /// Off lets go of the audio completely (for comparing).
    pub enabled: bool,
    pub devices: BTreeMap<String, Entry>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            enabled: true,
            devices: BTreeMap::new(),
        }
    }
}

/// What an output has chosen.
#[derive(Debug, Clone, Copy)]
pub struct Choice {
    pub kind: Kind,
    pub active: usize,
    pub bass: i32,
}

impl Choice {
    pub fn presets(&self) -> &'static [Preset] {
        self.kind.presets()
    }

    pub fn preset(&self) -> &'static Preset {
        &self.presets()[self.active]
    }
}

impl Config {
    pub fn load() -> Config {
        telmo_kit::state::load(STATE).unwrap_or_default()
    }

    /// Changes the file on disk under a lock and returns the result.
    pub fn update(change: impl FnOnce(&mut Config)) -> io::Result<Config> {
        telmo_kit::state::update(STATE, change)
    }

    /// The preset and bass nudge of an output; its kind's default when it has
    /// no entry (or one naming a preset that no longer exists).
    pub fn choice(&self, device: &Device) -> Option<Choice> {
        let target = device.eq.as_ref()?;
        let kind = Kind::of(&device.name, target);
        let entry = self.devices.get(&target.key);
        let active = entry
            .and_then(|e| kind.presets().iter().position(|p| p.name == e.preset))
            .unwrap_or(kind.default_preset());
        Some(Choice {
            kind,
            active,
            bass: entry.map_or(0, |e| e.bass),
        })
    }

    /// Records the preset and bass nudge of an output, with the filters they
    /// resolve to. Does nothing for an output that can't be equalized.
    pub fn set(&mut self, device: &Device, active: usize, bass: i32) {
        let Some(target) = &device.eq else { return };
        let kind = Kind::of(&device.name, target);
        let Some(preset) = kind.presets().get(active) else {
            return;
        };
        let resolved = resolve(preset, bass);
        self.devices.insert(
            target.key.clone(),
            Entry {
                name: device.name.clone(),
                preset: preset.name.to_string(),
                bass,
                preamp: resolved.preamp,
                filters: resolved.filters,
            },
        );
    }

    /// Gives every output without an entry its default. True when it added any.
    pub fn seed(&mut self, outputs: &[Device]) -> bool {
        let before = self.devices.len();
        for device in outputs {
            let Some(target) = &device.eq else { continue };
            if !self.devices.contains_key(&target.key) {
                let kind = Kind::of(&device.name, target);
                self.set(device, kind.default_preset(), 0);
            }
        }
        self.devices.len() != before
    }
}

/// Tells Telmo.app to read `eq.json` again. Best effort and off the caller's
/// thread: if the app isn't running it reads the file when it starts.
#[cfg(target_os = "macos")]
pub fn reload() {
    use std::{io::Write, os::unix::net::UnixStream, path::PathBuf, time::Duration};
    std::thread::spawn(|| {
        let path = std::env::var_os("TELMO_SOCKET")
            .map(PathBuf::from)
            .or_else(|| {
                let home = std::env::var_os("HOME")?;
                Some(PathBuf::from(home).join("Library/Application Support/Telmo/host.sock"))
            });
        let Some(Ok(mut socket)) = path.map(UnixStream::connect) else {
            return;
        };
        let _ = socket.set_write_timeout(Some(Duration::from_secs(2)));
        let _ = socket.write_all(b"eq reload\n");
    });
}

#[cfg(not(target_os = "macos"))]
pub fn reload() {}

#[cfg(test)]
mod tests {
    use super::*;

    fn device(name: &str, key: &str, builtin: bool) -> Device {
        Device {
            id: key.into(),
            name: name.into(),
            default: false,
            volume: Some(0.5),
            muted: false,
            bluetooth: false,
            profiles: Vec::new(),
            playing: false,
            eq: Some(EqTarget {
                key: key.into(),
                builtin,
            }),
        }
    }

    fn speakers() -> Device {
        device("MacBook Air Speakers", "BuiltInSpeakerDevice#ispk", true)
    }

    fn earfun() -> Device {
        device("EarFun Air Pro 4", "AA-BB-CC:output", false)
    }

    fn jack() -> Device {
        device("External Headphones", "BuiltInHeadphones#hdpn", true)
    }

    fn names(kind: Kind) -> Vec<&'static str> {
        kind.presets().iter().map(|p| p.name).collect()
    }

    #[test]
    fn each_kind_of_output_gets_its_list() {
        let kind = |d: &Device| Kind::of(&d.name, d.eq.as_ref().unwrap());
        assert_eq!(kind(&speakers()), Kind::Speakers);
        assert_eq!(kind(&earfun()), Kind::EarFun);
        assert_eq!(kind(&jack()), Kind::Wired);
        assert_eq!(kind(&device("LG UltraFine", "lg", false)), Kind::Generic);
        // A built-in output switched to the jack by its data source.
        assert_eq!(
            Kind::of("MacBook Air Headphones", &jack().eq.unwrap()),
            Kind::Wired
        );
    }

    #[test]
    fn the_lists_are_named_as_designed() {
        assert_eq!(
            names(Kind::Speakers),
            [
                "Speakers +",
                "Speakers ++",
                "Flat",
                "Bass boost",
                "Vocal",
                "Late night"
            ]
        );
        assert_eq!(
            names(Kind::EarFun),
            [
                "Flat",
                "Your EarFun EQ",
                "Bass boost",
                "Vocal",
                "Late night"
            ]
        );
        assert_eq!(
            names(Kind::Wired),
            [
                "As is",
                "+ Sub-bass",
                "+ Sub-bass ++",
                "Sub focus",
                "Warm",
                "Late night"
            ]
        );
        assert_eq!(
            names(Kind::Generic),
            ["Flat", "Bass boost", "Vocal", "Late night"]
        );
    }

    #[test]
    fn defaults_are_plus_flat_sub_bass_and_flat() {
        let default = |kind: Kind| kind.presets()[kind.default_preset()].name;
        assert_eq!(default(Kind::Speakers), "Speakers +");
        assert_eq!(default(Kind::EarFun), "Flat");
        assert_eq!(default(Kind::Wired), "+ Sub-bass");
        assert_eq!(default(Kind::Generic), "Flat");
    }

    #[test]
    fn a_preset_resolves_to_its_own_numbers() {
        let resolved = resolve(&Kind::Speakers.presets()[1], 0);
        assert_eq!(resolved.preamp, -4.5);
        assert_eq!(
            resolved.filters,
            [peak(170.0, 4.5, 0.7), high_pass(40.0, 0.7)]
        );
        let earfun = resolve(&Kind::EarFun.presets()[1], 0);
        assert_eq!(earfun.preamp, -10.0);
        assert_eq!(earfun.filters.len(), 3);
    }

    #[test]
    fn a_bass_boost_adds_a_shelf_and_lowers_the_preamp() {
        let plain = Kind::Speakers.presets()[0];
        let resolved = resolve(&plain, 2);
        assert_eq!(resolved.preamp, -5.0);
        assert_eq!(resolved.filters.last(), Some(&low_shelf(110.0, 2.0, 0.7)));
        assert_eq!(resolved.filters.len(), plain.filters.len() + 1);
    }

    #[test]
    fn a_bass_cut_keeps_the_preamp() {
        let resolved = resolve(&FLAT, -3);
        assert_eq!(resolved.preamp, 0.0);
        assert_eq!(resolved.filters, [low_shelf(110.0, -3.0, 0.7)]);
        assert_eq!(
            resolve(&FLAT, 0),
            Resolved {
                preamp: 0.0,
                filters: Vec::new()
            }
        );
    }

    #[test]
    fn an_output_without_an_entry_has_its_default() {
        let config = Config::default();
        let choice = config.choice(&speakers()).unwrap();
        assert_eq!(choice.preset().name, "Speakers +");
        assert_eq!(config.choice(&jack()).unwrap().preset().name, "+ Sub-bass");
        assert_eq!(config.choice(&earfun()).unwrap().preset().name, "Flat");
    }

    #[test]
    fn an_output_that_cannot_be_equalized_has_no_choice() {
        let mut linux = speakers();
        linux.eq = None;
        assert!(Config::default().choice(&linux).is_none());
        let mut config = Config::default();
        config.set(&linux, 1, 0);
        assert!(config.devices.is_empty());
    }

    #[test]
    fn seeding_writes_each_default_once_and_keeps_choices() {
        let mut config = Config::default();
        let outputs = [speakers(), earfun(), jack()];
        assert!(config.seed(&outputs));
        let speakers_entry = &config.devices["BuiltInSpeakerDevice#ispk"];
        assert_eq!(speakers_entry.preset, "Speakers +");
        assert_eq!(speakers_entry.preamp, -3.0);
        assert_eq!(speakers_entry.filters, [peak(160.0, 3.0, 0.7)]);
        assert_eq!(
            config.devices["BuiltInHeadphones#hdpn"].preset,
            "+ Sub-bass"
        );
        assert!(config.devices["AA-BB-CC:output"].filters.is_empty());

        config.set(&speakers(), 3, 1);
        assert!(!config.seed(&outputs));
        assert_eq!(
            config.devices["BuiltInSpeakerDevice#ispk"].preset,
            "Bass boost"
        );
    }

    #[test]
    fn set_remembers_the_nudge_per_output() {
        let mut config = Config::default();
        config.set(&speakers(), 0, 2);
        let choice = config.choice(&speakers()).unwrap();
        assert_eq!((choice.active, choice.bass), (0, 2));
        assert_eq!(config.devices["BuiltInSpeakerDevice#ispk"].preamp, -5.0);
        assert_eq!(config.choice(&jack()).unwrap().bass, 0);
    }

    #[test]
    fn a_preset_that_no_longer_exists_falls_back_to_the_default() {
        let mut config = Config::default();
        config.set(&speakers(), 1, 0);
        config
            .devices
            .get_mut("BuiltInSpeakerDevice#ispk")
            .unwrap()
            .preset = "Gone".into();
        assert_eq!(
            config.choice(&speakers()).unwrap().preset().name,
            "Speakers +"
        );
    }

    #[test]
    fn the_file_has_the_shape_telmo_app_reads() {
        let mut config = Config::default();
        config.set(&speakers(), 0, 0);
        let json: serde_json::Value = serde_json::to_value(&config).unwrap();
        assert_eq!(
            json,
            serde_json::json!({
                "enabled": true,
                "devices": {"BuiltInSpeakerDevice#ispk": {
                    "name": "MacBook Air Speakers",
                    "preset": "Speakers +",
                    "bass": 0,
                    "preamp": -3.0,
                    "filters": [{"type": "peak", "freq": 160.0, "gain": 3.0, "q": 0.7}],
                }},
            })
        );
        let shelf = serde_json::to_value(low_shelf(100.0, 2.0, 0.7)).unwrap();
        assert_eq!(shelf["type"], "lowshelf");
        assert_eq!(
            serde_json::to_value(high_shelf(1.0, 1.0, 1.0)).unwrap()["type"],
            "highshelf"
        );
        assert_eq!(
            serde_json::to_value(high_pass(40.0, 0.7)).unwrap()["type"],
            "highpass"
        );
    }

    #[test]
    fn the_file_survives_a_round_trip_and_a_missing_one() {
        let mut config = Config {
            enabled: false,
            ..Config::default()
        };
        config.seed(&[speakers(), earfun(), jack()]);
        config.set(&jack(), 3, -2);
        let text = serde_json::to_string(&config).unwrap();
        assert_eq!(serde_json::from_str::<Config>(&text).unwrap(), config);
        // Missing fields mean the defaults: on, no devices.
        assert_eq!(
            serde_json::from_str::<Config>("{}").unwrap(),
            Config::default()
        );
    }
}
