//! Plain data shared by the UI and every backend.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Snapshot {
    /// Screens, then the keyboard backlight when the machine has one.
    pub displays: Vec<Display>,
    pub night: Night,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Kind {
    Builtin,
    External,
    Keyboard,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Display {
    pub id: String,
    pub name: String,
    pub kind: Kind,
    /// 0.0–1.0. None when the system gives no way to change it.
    pub brightness: Option<f32>,
    pub info: Info,
    /// Sizes the display can be switched to. Empty when it has no choice.
    pub modes: Vec<Mode>,
    /// Said under the size choices, e.g. that they do not survive a reload.
    #[serde(default)]
    pub modes_note: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Info {
    /// "2560 × 1440", in pixels.
    pub resolution: Option<String>,
    pub refresh: Option<f32>,
    pub scale: Option<f32>,
    pub connection: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Mode {
    pub id: String,
    /// What it looks like: "1440 × 900", or "1.5x" on Linux.
    pub label: String,
    /// "Larger text", "Default", "More space", or empty.
    pub note: String,
    pub current: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Night {
    pub available: bool,
    pub on: bool,
    /// Warmth, 0.0 (none) to 1.0 (most).
    pub warmth: f32,
}
