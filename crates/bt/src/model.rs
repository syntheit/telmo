//! Plain data shared by the UI and every backend.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Snapshot {
    /// None when there is no adapter at all.
    pub adapter: Option<Adapter>,
    /// Paired devices plus, while scanning, newly discovered ones.
    pub devices: Vec<Device>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Adapter {
    /// Shown in the header, e.g. "hci0" or "Apple BCM4388". Not shown today.
    pub name: String,
    pub powered: bool,
    pub discovering: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Kind {
    Headphones,
    Speaker,
    Keyboard,
    Mouse,
    Gamepad,
    Phone,
    Computer,
    Other,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Device {
    /// Bluetooth address, e.g. "4C:87:5D:1A:0F:22".
    pub id: String,
    pub name: String,
    pub kind: Kind,
    pub paired: bool,
    pub connected: bool,
    pub trusted: bool,
    pub battery: Option<Battery>,
    /// Signal while discovering, in dBm.
    pub rssi: Option<i16>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Battery {
    Single(u8),
    /// Earbuds report each side and the case separately (AirPods on macOS).
    Buds {
        left: Option<u8>,
        right: Option<u8>,
        case: Option<u8>,
    },
}

/// What a device asks for while pairing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PairPrompt {
    /// Type this code on the device.
    DisplayPasskey(u32),
    /// Does the device show this code? yes / no.
    Confirm(u32),
    /// Enter the PIN the device shows (old devices).
    EnterPin,
}
