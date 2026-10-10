//! What the user picked before: which app a query meant, and how often and
//! how recently each app was opened. Kept in `~/.local/state/telmo/launcher.json`.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Queries remembered; the oldest go first.
const MAX_PICKS: usize = 400;
const MAX_APPS: usize = 400;

const HOUR: u64 = 3600;
const DAY: u64 = 24 * HOUR;
const WEEK: u64 = 7 * DAY;

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Pick {
    pub app: String,
    pub at: u64,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Use {
    pub count: u32,
    pub last: u64,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct History {
    /// Normalised query -> the app opened from it last.
    pub picks: BTreeMap<String, Pick>,
    pub apps: BTreeMap<String, Use>,
}

/// The state file's name under `telmo_kit::state`.
pub const FILE: &str = "launcher";

/// Lowercase, one space between words: `" VSC "` and `vsc` are the same query.
pub fn normalize(query: &str) -> String {
    query
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

impl History {
    pub fn load() -> Self {
        telmo_kit::state::load(FILE).unwrap_or_default()
    }

    /// Notes that `app` was opened, from `query` if one was typed.
    pub fn record(&mut self, query: &str, app: &str, now: u64) {
        let used = self.apps.entry(app.to_string()).or_default();
        used.count = used.count.saturating_add(1);
        used.last = now;
        let query = normalize(query);
        if !query.is_empty() {
            self.picks.insert(
                query,
                Pick {
                    app: app.to_string(),
                    at: now,
                },
            );
        }
        prune(&mut self.picks, MAX_PICKS, |p| p.at);
        prune(&mut self.apps, MAX_APPS, |u| u.last);
    }

    /// The app this exact query led to before.
    pub fn picked(&self, query: &str) -> Option<&str> {
        self.picks.get(&normalize(query)).map(|p| p.app.as_str())
    }

    /// Frequency weighted by recency, as zoxide does: opens count 4× within
    /// an hour, 2× within a day, half within a week, a quarter after that.
    pub fn frecency(&self, app: &str, now: u64) -> f64 {
        let Some(used) = self.apps.get(app) else {
            return 0.0;
        };
        let age = now.saturating_sub(used.last);
        let weight = match age {
            a if a < HOUR => 4.0,
            a if a < DAY => 2.0,
            a if a < WEEK => 0.5,
            _ => 0.25,
        };
        f64::from(used.count) * weight
    }

    /// App ids, most recently opened first.
    pub fn recent(&self) -> Vec<&str> {
        let mut apps: Vec<_> = self.apps.iter().collect();
        apps.sort_by(|a, b| b.1.last.cmp(&a.1.last).then(a.0.cmp(b.0)));
        apps.into_iter().map(|(id, _)| id.as_str()).collect()
    }
}

/// Drops the entries with the smallest timestamp until `max` are left.
fn prune<V>(map: &mut BTreeMap<String, V>, max: usize, at: impl Fn(&V) -> u64) {
    while map.len() > max {
        let Some(oldest) = map
            .iter()
            .min_by_key(|(_, v)| at(v))
            .map(|(k, _)| k.clone())
        else {
            return;
        };
        map.remove(&oldest);
    }
}

/// Writes the open to the state file, under its lock. Failure only costs the
/// learning, so callers may ignore it.
pub fn save_open(query: &str, app: &str, now: u64) -> std::io::Result<()> {
    telmo_kit::state::update(FILE, |h: &mut History| h.record(query, app, now)).map(|_| ())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_pick_is_learned_for_that_exact_query() {
        let mut h = History::default();
        h.record("  VSC ", "com.vsc", 100);
        assert_eq!(h.picked("vsc"), Some("com.vsc"));
        assert_eq!(h.picked("vs"), None);
        h.record("vsc", "com.other", 200);
        assert_eq!(h.picked("VSC"), Some("com.other"), "the latest choice wins");
    }

    #[test]
    fn opening_without_a_query_still_counts() {
        let mut h = History::default();
        h.record("", "a", 10);
        h.record("", "a", 20);
        h.record("", "b", 30);
        assert!(h.picks.is_empty());
        assert_eq!(h.apps["a"], Use { count: 2, last: 20 });
        assert_eq!(h.recent(), ["b", "a"]);
    }

    #[test]
    fn frecency_decays_in_buckets() {
        let mut h = History::default();
        h.apps.insert(
            "a".into(),
            Use {
                count: 10,
                last: 1_000_000,
            },
        );
        assert_eq!(h.frecency("a", 1_000_000 + 60), 40.0);
        assert_eq!(h.frecency("a", 1_000_000 + 2 * HOUR), 20.0);
        assert_eq!(h.frecency("a", 1_000_000 + 2 * DAY), 5.0);
        assert_eq!(h.frecency("a", 1_000_000 + 2 * WEEK), 2.5);
        assert_eq!(h.frecency("nope", 5), 0.0);
    }

    #[test]
    fn the_oldest_picks_are_forgotten() {
        let mut h = History::default();
        for i in 0..(MAX_PICKS + 5) {
            h.record(&format!("q{i}"), "app", i as u64);
        }
        assert_eq!(h.picks.len(), MAX_PICKS);
        assert!(!h.picks.contains_key("q0"));
        assert!(h.picks.contains_key(&format!("q{}", MAX_PICKS + 4)));
    }

    #[test]
    fn it_round_trips_through_json() {
        let mut h = History::default();
        h.record("zen", "org.zen", 5);
        let text = serde_json::to_string(&h).unwrap();
        assert_eq!(serde_json::from_str::<History>(&text).unwrap(), h);
        assert_eq!(
            serde_json::from_str::<History>("{}").unwrap(),
            History::default()
        );
    }

    #[test]
    fn the_state_file_round_trips_on_disk() {
        let dir = std::env::temp_dir().join(format!("telmo-launcher-state-{}", std::process::id()));
        // Only this test reads the state directory from the environment.
        // SAFETY: nothing else in this test binary reads or sets XDG_STATE_HOME.
        unsafe { std::env::set_var("XDG_STATE_HOME", &dir) };
        save_open("tg", "org.telegram", 42).expect("save");
        let loaded = History::load();
        assert_eq!(loaded.picked("tg"), Some("org.telegram"));
        assert_eq!(loaded.apps["org.telegram"].count, 1);
        let _ = std::fs::remove_dir_all(dir);
    }
}
