//! `~/.config/telmo/clipboard.json`, written by Nix.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Config {
    /// Unpinned items kept.
    pub max_items: usize,
    /// Unpinned items older than this are dropped.
    pub max_days: u64,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            max_items: 50,
            max_days: 30,
        }
    }
}

pub fn config_path() -> Option<PathBuf> {
    Some(telmo_kit::dirs::config()?.join("telmo/clipboard.json"))
}

/// `$XDG_DATA_HOME/telmo/clipboard`, falling back to `~/.local/share`.
pub fn data_dir() -> Option<PathBuf> {
    Some(telmo_kit::dirs::data()?.join("telmo/clipboard"))
}

/// A missing file means the defaults; a broken one is an error worth showing.
pub fn load() -> Result<Config, String> {
    let Some(path) = config_path() else {
        return Ok(Config::default());
    };
    let text = match std::fs::read_to_string(&path) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Config::default()),
        Err(e) => return Err(format!("Can't read {}: {e}", path.display())),
    };
    serde_json::from_str(&text).map_err(|e| {
        format!(
            "{} isn't valid: {e}. Fix or delete the file.",
            path.display()
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn partial_files_fill_in_defaults() {
        let config: Config = serde_json::from_str(r#"{ "maxDays": 7 }"#).unwrap();
        assert_eq!(config.max_days, 7);
        assert_eq!(config.max_items, 50);
    }
}
