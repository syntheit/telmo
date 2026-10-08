//! What the user picked (effect, logo). Order of precedence: the saved choice,
//! then the optional Nix-written config, then the defaults.

use crate::effects::{LogoKind, NAMES};
use serde::{Deserialize, Serialize};
use std::{io, path::PathBuf};

const STATE: &str = "system";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Prefs {
    pub effect: String,
    pub logo: LogoKind,
}

/// `~/.config/telmo/system.json`; every field is optional.
#[derive(Debug, Default, Deserialize)]
pub struct Config {
    effect: Option<String>,
    logo: Option<LogoKind>,
    effects: Option<Vec<String>>,
    rebuild: Option<Vec<String>>,
}

#[derive(Debug, PartialEq)]
pub struct Resolved {
    pub prefs: Prefs,
    /// The effects the arrow keys cycle through, in order.
    pub cycle: Vec<String>,
    /// The command `u` runs (without elevation), if configured.
    pub rebuild: Option<Vec<String>>,
}

pub fn resolve(state: Option<Prefs>, config: Option<Config>) -> Resolved {
    let config = config.unwrap_or_default();
    let mut cycle: Vec<String> = config
        .effects
        .iter()
        .flatten()
        .filter(|name| NAMES.contains(&name.as_str()))
        .cloned()
        .collect();
    if cycle.is_empty() {
        cycle = NAMES.iter().map(|name| name.to_string()).collect();
    }
    let known = |name: &&String| cycle.contains(name);
    let effect = state
        .as_ref()
        .map(|s| &s.effect)
        .filter(known)
        .or(config.effect.as_ref().filter(known))
        .unwrap_or(&cycle[0])
        .clone();
    let logo = state
        .map(|s| s.logo)
        .or(config.logo)
        .unwrap_or_else(LogoKind::native);
    Resolved {
        prefs: Prefs { effect, logo },
        cycle,
        rebuild: config.rebuild.filter(|command| !command.is_empty()),
    }
}

pub fn load() -> Resolved {
    resolve(telmo_kit::state::load(STATE), load_config())
}

pub fn save(prefs: &Prefs) -> io::Result<()> {
    telmo_kit::state::save(STATE, prefs)
}

fn config_path() -> Option<PathBuf> {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))?;
    Some(base.join("telmo/system.json"))
}

fn load_config() -> Option<Config> {
    let bytes = std::fs::read(config_path()?).ok()?;
    serde_json::from_slice(&bytes).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn prefs(effect: &str, logo: LogoKind) -> Prefs {
        Prefs {
            effect: effect.into(),
            logo,
        }
    }

    fn config(json: &str) -> Option<Config> {
        serde_json::from_str(json).ok()
    }

    #[test]
    fn defaults() {
        let r = resolve(None, None);
        assert_eq!(r.prefs, prefs(NAMES[0], LogoKind::native()));
        assert_eq!(r.cycle.len(), NAMES.len());
    }

    #[test]
    fn config_beats_default() {
        let r = resolve(None, config(r#"{"effect":"fire","logo":"nix"}"#));
        assert_eq!(r.prefs, prefs("fire", LogoKind::Nix));
    }

    #[test]
    fn state_beats_config() {
        let state = prefs("snow", LogoKind::Apple);
        let r = resolve(
            Some(state.clone()),
            config(r#"{"effect":"fire","logo":"nix"}"#),
        );
        assert_eq!(r.prefs, state);
    }

    #[test]
    fn unknown_effect_falls_back() {
        let r = resolve(
            Some(prefs("nope", LogoKind::Nix)),
            config(r#"{"effect":"fire"}"#),
        );
        assert_eq!(r.prefs, prefs("fire", LogoKind::Nix));
        let r = resolve(
            Some(prefs("nope", LogoKind::Nix)),
            config(r#"{"effect":"nope"}"#),
        );
        assert_eq!(r.prefs.effect, NAMES[0]);
    }

    #[test]
    fn subset_is_respected() {
        let r = resolve(
            Some(prefs("rain", LogoKind::Nix)),
            config(r#"{"effects":["fire","bogus","snow"]}"#),
        );
        assert_eq!(r.cycle, ["fire", "snow"]);
        assert_eq!(r.prefs.effect, "fire");
    }

    #[test]
    fn empty_subset_means_all() {
        let r = resolve(None, config(r#"{"effects":["bogus"]}"#));
        assert_eq!(r.cycle.len(), NAMES.len());
    }
}
