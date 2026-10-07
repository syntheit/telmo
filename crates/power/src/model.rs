//! Plain data shared by the UI and every backend.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Snapshot {
    /// None on a desktop.
    pub battery: Option<Battery>,
    /// None when the system has no power mode to change.
    pub mode: Option<ModeSetting>,
    pub keep_awake: KeepAwake,
    /// Whether this system can list the apps using the most energy (macOS).
    pub lists_energy_users: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Battery {
    pub percent: u8,
    pub state: ChargeState,
    /// Until empty while on battery, until full while charging.
    pub minutes: Option<u32>,
    /// E.g. "96 W adapter"; None while unplugged.
    pub source: Option<String>,
    pub health: Option<Health>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ChargeState {
    Charging,
    OnBattery,
    Full,
    NotCharging,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Health {
    /// Capacity compared to when new, in percent.
    pub max_capacity: Option<u8>,
    pub cycles: Option<u32>,
    pub condition: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PowerMode {
    Saver,
    Balanced,
    Performance,
}

impl PowerMode {
    pub fn label(self) -> &'static str {
        match self {
            PowerMode::Saver => "power saver",
            PowerMode::Balanced => "balanced",
            PowerMode::Performance => "performance",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ModeSetting {
    pub current: PowerMode,
    pub available: Vec<PowerMode>,
}

impl ModeSetting {
    /// macOS only has Low Power Mode on or off; Linux offers profiles.
    pub fn is_toggle(&self) -> bool {
        !self.available.contains(&PowerMode::Performance)
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum KeepAwake {
    #[default]
    Off,
    Timed {
        minutes_left: u32,
    },
    Indefinite,
}

/// What the user can pick in the Keep awake dialog.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AwakeChoice {
    Off,
    Minutes(u32),
    Indefinite,
}

impl AwakeChoice {
    pub const ALL: [AwakeChoice; 5] = [
        AwakeChoice::Off,
        AwakeChoice::Minutes(30),
        AwakeChoice::Minutes(60),
        AwakeChoice::Minutes(120),
        AwakeChoice::Indefinite,
    ];

    pub fn label(self) -> &'static str {
        match self {
            AwakeChoice::Off => "Off",
            AwakeChoice::Minutes(30) => "30 min",
            AwakeChoice::Minutes(60) => "1 hour",
            AwakeChoice::Minutes(120) => "2 hours",
            AwakeChoice::Minutes(_) => "Timed",
            AwakeChoice::Indefinite => "Until turned off",
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct EnergyUser {
    pub name: String,
    /// macOS "power" score; only the ratio between apps matters.
    pub power: f32,
}

/// "4h 12m", or "35m" under an hour.
pub fn duration(minutes: u32) -> String {
    match (minutes / 60, minutes % 60) {
        (0, m) => format!("{m}m"),
        (h, 0) => format!("{h}h"),
        (h, m) => format!("{h}h {m:02}m"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn durations() {
        assert_eq!(duration(35), "35m");
        assert_eq!(duration(252), "4h 12m");
        assert_eq!(duration(65), "1h 05m");
        assert_eq!(duration(120), "2h");
    }
}
