//! `~/.config/telmo/clock-config.json`, written by Nix: the cities of the
//! World tab. (The state lives in `~/.local/state/telmo/clock.json`.)

use jiff::tz::TimeZone;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct City {
    pub name: String,
    /// An IANA zone name such as `America/Argentina/Buenos_Aires`.
    pub zone: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub cities: Vec<City>,
}

impl Default for Config {
    fn default() -> Self {
        let city = |name: &str, zone: &str| City {
            name: name.into(),
            zone: zone.into(),
        };
        Self {
            cities: vec![
                city("Buenos Aires", "America/Argentina/Buenos_Aires"),
                city("San Francisco", "America/Los_Angeles"),
                city("New York", "America/New_York"),
                city("London", "Europe/London"),
                city("Tokyo", "Asia/Tokyo"),
            ],
        }
    }
}

/// A city whose zone has been looked up; None when the name is not a zone.
pub struct Place {
    pub name: String,
    pub zone_name: String,
    pub zone: Option<TimeZone>,
}

impl Config {
    pub fn places(&self) -> Vec<Place> {
        self.cities
            .iter()
            .map(|city| Place {
                name: city.name.clone(),
                zone_name: city.zone.clone(),
                zone: TimeZone::get(&city.zone).ok(),
            })
            .collect()
    }
}

pub fn path() -> Option<PathBuf> {
    let dir = std::env::var_os("XDG_CONFIG_HOME")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))?;
    Some(dir.join("telmo/clock-config.json"))
}

/// A missing file means the default cities; a broken one is an error worth showing.
pub fn load() -> Result<Config, String> {
    let Some(path) = path() else {
        return Ok(Config::default());
    };
    let text = match std::fs::read_to_string(&path) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Config::default()),
        Err(e) => return Err(format!("Can't read {}: {e}", path.display())),
    };
    serde_json::from_str(&text).map_err(|e| format!("{} is not valid: {e}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_resolve() {
        let places = Config::default().places();
        assert_eq!(places.len(), 5);
        assert!(places.iter().all(|p| p.zone.is_some()));
    }

    #[test]
    fn reads_what_nix_writes_and_flags_unknown_zones() {
        let config: Config = serde_json::from_str(
            r#"{"cities":[{"name":"Lisbon","zone":"Europe/Lisbon"},{"name":"Oz","zone":"Nowhere/Land"}]}"#,
        )
        .expect("config");
        let places = config.places();
        assert!(places[0].zone.is_some());
        assert!(places[1].zone.is_none());
        let empty: Config = serde_json::from_str("{}").expect("empty");
        assert_eq!(empty, Config::default());
    }
}
