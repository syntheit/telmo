//! The optional privileged helper (`telmo-helper`) reads and changes per-network
//! auto-join, which macOS keeps in a root-only file. Only the signed host app may
//! talk to it, so we send `helper <line>` to the host, which forwards the line.
//! Without the helper every call fails and the caller keeps its old behavior.

use super::host;
use std::collections::HashMap;
use std::sync::{LazyLock, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

/// Ask the helper again no more often than this.
const FRESH: Duration = Duration::from_secs(5);

#[derive(Default)]
struct Cache {
    networks: HashMap<String, bool>,
    fetched: Option<Instant>,
    fetching: bool,
}

static CACHE: LazyLock<Mutex<Cache>> = LazyLock::new(Mutex::default);

fn cache() -> MutexGuard<'static, Cache> {
    CACHE.lock().unwrap_or_else(PoisonError::into_inner)
}

/// The last known auto-join state per saved network. Never blocks.
pub fn cached() -> HashMap<String, bool> {
    cache().networks.clone()
}

/// True once if the cache is old and nobody is refreshing it; the caller must
/// then call `refresh`.
pub fn claim_refresh() -> bool {
    let mut cache = cache();
    let stale = cache.fetched.is_none_or(|at| at.elapsed() >= FRESH);
    let claim = stale && !cache.fetching;
    cache.fetching |= claim;
    claim
}

/// Ask the helper. Returns whether the answer differs from the cache.
/// An unreachable helper counts as an empty list.
pub fn refresh() -> bool {
    let fetched = host::command("helper autojoin-list", host::QUICK)
        .and_then(|reply| parse_list(&reply))
        .unwrap_or_default();
    let mut cache = cache();
    let changed = cache.networks != fetched;
    cache.networks = fetched;
    cache.fetched = Some(Instant::now());
    cache.fetching = false;
    changed
}

pub fn set(ssid: &str, on: bool) -> Result<(), String> {
    let line = set_line(ssid, on)?;
    let reply = host::command(&format!("helper {line}"), host::QUICK)?;
    if reply != "ok" {
        return Err("The helper gave an unexpected answer.".to_string());
    }
    cache().networks.insert(ssid.to_string(), on);
    Ok(())
}

fn set_line(ssid: &str, on: bool) -> Result<String, String> {
    if ssid.is_empty() || ssid.chars().any(char::is_control) {
        return Err("That network name can't be sent to the helper.".to_string());
    }
    Ok(format!(
        "autojoin-set {ssid} {}",
        if on { "on" } else { "off" }
    ))
}

fn parse_list(reply: &str) -> Result<HashMap<String, bool>, String> {
    serde_json::from_str(reply).map_err(|_| "The helper gave an unexpected list.".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_set_lines() {
        assert_eq!(
            set_line("Home Net", false).unwrap(),
            "autojoin-set Home Net off"
        );
        assert_eq!(set_line("cafe", true).unwrap(), "autojoin-set cafe on");
    }

    #[test]
    fn refuses_names_that_could_break_the_line() {
        assert!(set_line("a\nb", true).is_err());
        assert!(set_line("", true).is_err());
    }

    #[test]
    fn parses_lists() {
        let map = parse_list(r#"{"Home":false,"Cafe":true}"#).unwrap();
        assert!(!map["Home"]);
        assert!(map["Cafe"]);
        assert!(parse_list("error nope").is_err());
    }
}
