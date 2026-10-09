//! Plain data shared by the UI and every backend.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Snapshot {
    pub outputs: Vec<Device>,
    pub inputs: Vec<Device>,
    /// Apps currently playing. Empty when the OS can't list them.
    pub streams: Vec<Stream>,
    pub caps: Caps,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Caps {
    pub per_app: bool,
    /// Bluetooth profiles (best sound vs headset) can be switched.
    pub profiles: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Direction {
    Output,
    Input,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Device {
    pub id: String,
    pub name: String,
    pub default: bool,
    /// 0.0–1.0 (Linux may go up to 1.5). None when there's no software volume.
    pub volume: Option<f32>,
    pub muted: bool,
    pub bluetooth: bool,
    pub profiles: Vec<Profile>,
    /// Sound is coming out of (or going into) this device right now.
    #[serde(default)]
    pub playing: bool,
    /// Set when the output can be equalized (macOS only for now).
    #[serde(default)]
    pub eq: Option<EqTarget>,
}

/// How the EQ recognizes an output.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EqTarget {
    /// The device UID, plus `#<data source>` on a built-in device, so the
    /// speakers and the headphone jack keep separate presets. Telmo.app builds
    /// the same key for the output it equalizes.
    pub key: String,
    pub builtin: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Profile {
    pub id: String,
    /// "Best sound (no mic)", "Headset (mic on, lower quality)".
    pub name: String,
    pub active: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Stream {
    pub id: String,
    pub app: String,
    pub volume: f32,
    pub muted: bool,
    /// Output device id this app plays to.
    pub device: Option<String>,
    #[serde(default)]
    pub playing: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Target {
    Device(Direction, String),
    Stream(String),
}

/// Where song recognition listens: what the computer plays, or the room.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    Desktop,
    Mic,
}
