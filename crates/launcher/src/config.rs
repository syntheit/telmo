//! `~/.config/telmo/launcher.json`, written by Nix.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::PathBuf;

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Config {
    /// One letter per app: `{"t": "Ghostty"}`. The letter is a global hotkey
    /// and, typed in the launcher, puts that app on top.
    pub keys: BTreeMap<String, String>,
    /// What the list shows before the letter: `fn` gives `fn T`.
    pub hotkey_label: String,
    /// Web search engines by short name; `%s` stands for the search words.
    pub search: BTreeMap<String, String>,
    /// The engine `?words` uses when no engine is named. Default: `nix`,
    /// else the first by name.
    pub default_search: Option<String>,
}

impl Config {
    /// The key letter of an app called `name`, lowercase.
    pub fn key_of(&self, name: &str) -> Option<char> {
        self.keys
            .iter()
            .find(|(_, app)| app.eq_ignore_ascii_case(name))
            .and_then(|(key, _)| key.chars().next())
            .map(|c| c.to_ascii_lowercase())
    }

    /// `fn T`, or just `T` without a label.
    pub fn hotkey_text(&self, key: char) -> String {
        let key = key.to_ascii_uppercase();
        if self.hotkey_label.is_empty() {
            key.to_string()
        } else {
            format!("{} {key}", self.hotkey_label)
        }
    }
}

fn home_dir(var: &str, fallback: &str) -> Option<PathBuf> {
    std::env::var_os(var)
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(fallback)))
}

pub fn config_path() -> Option<PathBuf> {
    Some(home_dir("XDG_CONFIG_HOME", ".config")?.join("telmo/launcher.json"))
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
        let config: Config =
            serde_json::from_str(r#"{ "keys": { "t": "Ghostty" }, "hotkeyLabel": "fn" }"#).unwrap();
        assert!(config.search.is_empty());
        assert_eq!(config.key_of("ghostty"), Some('t'));
        assert_eq!(config.hotkey_text('t'), "fn T");
        assert_eq!(config.key_of("Zen"), None);
    }

    #[test]
    fn no_label_shows_the_bare_letter() {
        assert_eq!(Config::default().hotkey_text('w'), "W");
    }
}
