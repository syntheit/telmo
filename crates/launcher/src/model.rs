//! What the launcher knows about an installed app.

use serde::{Deserialize, Serialize};

/// One launchable app. The cache file is a list of these.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AppEntry {
    /// Stable across rebuilds: the bundle identifier (macOS) or the desktop
    /// file ID (Linux). History and "running" are keyed by it.
    pub id: String,
    pub name: String,
    /// Other names the app answers to (the plist's names, a generic name).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub aliases: Vec<String>,
    /// The `.app` bundle (macOS) or the `.desktop` file (Linux).
    pub path: String,
    /// Linux: the `Icon=` value, a name or an absolute path.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,
    /// Linux: the `Exec=` line, field codes still in it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exec: Option<String>,
    /// Linux: `StartupWMClass=`, to find the app's window.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub wm_class: Option<String>,
    /// Linux: `Terminal=true`.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub terminal: bool,
}

impl AppEntry {
    pub fn new(id: &str, name: &str, path: &str) -> Self {
        Self {
            id: id.into(),
            name: name.into(),
            aliases: Vec::new(),
            path: path.into(),
            icon: None,
            exec: None,
            wm_class: None,
            terminal: false,
        }
    }

    /// Whether `wanted` is this app's name or one of its aliases, ignoring case.
    pub fn is_called(&self, wanted: &str) -> bool {
        let wanted = wanted.trim();
        std::iter::once(&self.name)
            .chain(&self.aliases)
            .any(|n| n.eq_ignore_ascii_case(wanted))
    }
}

/// Finds an app by the name a hotkey or `open` gives it.
pub fn find_named<'a>(apps: &'a [AppEntry], wanted: &str) -> Option<&'a AppEntry> {
    apps.iter()
        .find(|a| a.name.eq_ignore_ascii_case(wanted.trim()))
        .or_else(|| apps.iter().find(|a| a.is_called(wanted)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_match_without_regard_to_case_and_aliases_count() {
        let mut code = AppEntry::new("com.vsc", "Visual Studio Code", "/A/Visual Studio Code.app");
        code.aliases.push("Code".into());
        let apps = vec![AppEntry::new("com.zen", "Zen", "/A/Zen.app"), code];
        assert_eq!(
            find_named(&apps, "visual studio code").unwrap().id,
            "com.vsc"
        );
        assert_eq!(find_named(&apps, " Code ").unwrap().id, "com.vsc");
        assert!(find_named(&apps, "Nope").is_none());
    }
}
