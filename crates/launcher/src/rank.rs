//! Matching and ordering. Order of preference: an exact letter key, the
//! app this exact query led to before, a name that starts with the query, a
//! word that does, the word initials (`vsc`), then fuzzy. Frecency only
//! reorders within one of these steps, so use can't bury a better match.

use crate::history::History;
use crate::model::AppEntry;
use nucleo_matcher::{
    Config, Matcher, Utf32Str,
    pattern::{AtomKind, CaseMatching, Normalization, Pattern},
};

const KEY: i64 = 10_000;
const LEARNED: i64 = 8_000;
const PREFIX: i64 = 3_000;
const WORD: i64 = 2_500;
const INITIALS: i64 = 2_000;
const FUZZY: i64 = 500;
/// The most a frequently used app can gain: less than the gap between steps.
const MAX_BOOST: f64 = 300.0;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Match {
    pub score: i64,
    /// Character positions in the name to highlight.
    pub hits: Vec<u32>,
}

pub struct Ranker {
    matcher: Matcher,
}

impl Default for Ranker {
    fn default() -> Self {
        Self::new()
    }
}

fn is_separator(c: char) -> bool {
    matches!(c, ' ' | '-' | '.' | '_' | '/' | '(' | ')' | ':')
}

/// Character positions where a word starts.
fn word_starts(name: &[char]) -> Vec<usize> {
    (0..name.len())
        .filter(|&i| {
            name[i].is_alphanumeric()
                && (i == 0
                    || is_separator(name[i - 1])
                    || (name[i - 1].is_lowercase() && name[i].is_uppercase()))
        })
        .collect()
}

fn lower(chars: &[char]) -> Vec<char> {
    chars
        .iter()
        .map(|c| c.to_lowercase().next().unwrap_or(*c))
        .collect()
}

impl Ranker {
    pub fn new() -> Self {
        Self {
            matcher: Matcher::new(Config::DEFAULT),
        }
    }

    /// How well `query` matches `name`, without any history.
    pub fn text(&mut self, name: &str, query: &str) -> Option<Match> {
        let query: Vec<char> = lower(&query.trim().chars().collect::<Vec<_>>());
        if query.is_empty() {
            return Some(Match {
                score: 0,
                hits: Vec::new(),
            });
        }
        let original: Vec<char> = name.chars().collect();
        let name_lower = lower(&original);
        let length_cost = original.len().min(60) as i64 * 10;
        let spans = |from: usize| (from as u32..(from + query.len()) as u32).collect::<Vec<_>>();

        if name_lower.starts_with(&query) {
            return Some(Match {
                score: PREFIX - length_cost,
                hits: spans(0),
            });
        }
        let starts = word_starts(&original);
        // A later word that starts with the query: "stud" in "Visual Studio Code".
        for &start in &starts {
            if name_lower[start..].starts_with(&query) {
                return Some(Match {
                    score: WORD - length_cost - start as i64,
                    hits: spans(start),
                });
            }
        }
        if query.len() >= 2 && !query.contains(&' ') {
            let mut hits = Vec::new();
            let mut at = 0;
            for &start in &starts {
                if at < query.len() && name_lower[start] == query[at] {
                    hits.push(start as u32);
                    at += 1;
                }
            }
            if at == query.len() {
                return Some(Match {
                    score: INITIALS - length_cost,
                    hits,
                });
            }
        }
        if query.len() >= 2 {
            return self.fuzzy(name, &query.iter().collect::<String>(), length_cost);
        }
        None
    }

    fn fuzzy(&mut self, name: &str, query: &str, length_cost: i64) -> Option<Match> {
        let pattern = Pattern::new(
            query,
            CaseMatching::Ignore,
            Normalization::Smart,
            AtomKind::Fuzzy,
        );
        let mut buffer = Vec::new();
        let haystack = Utf32Str::new(name, &mut buffer);
        let mut hits = Vec::new();
        let score = pattern.indices(haystack, &mut self.matcher, &mut hits)?;
        hits.sort_unstable();
        hits.dedup();
        Some(Match {
            score: FUZZY + i64::from(score.min(400)) - length_cost / 4,
            hits,
        })
    }

    /// The apps for a typed query, best first. `key_of` gives an app's letter.
    pub fn apps(
        &mut self,
        apps: &[AppEntry],
        query: &str,
        history: &History,
        key_of: impl Fn(&AppEntry) -> Option<char>,
        now: u64,
        limit: usize,
    ) -> Vec<(usize, Vec<u32>)> {
        let learned = history.picked(query);
        let mut one_letter = query.trim().chars();
        let letter = match (one_letter.next(), one_letter.next()) {
            (Some(c), None) => Some(c.to_ascii_lowercase()),
            _ => None,
        };
        let mut scored: Vec<(i64, usize, Vec<u32>)> = Vec::new();
        for (index, app) in apps.iter().enumerate() {
            // The name counts for highlighting; an alias only for finding.
            let mut found = self.text(&app.name, query);
            if found.is_none() {
                found = app
                    .aliases
                    .iter()
                    .filter_map(|alias| self.text(alias, query))
                    .max_by_key(|m| m.score)
                    .map(|m| Match {
                        score: m.score - 200,
                        hits: Vec::new(),
                    });
            }
            let by_key = letter.is_some() && key_of(app) == letter;
            let by_pick = learned == Some(app.id.as_str());
            if found.is_none() && !by_pick && !by_key {
                continue;
            }
            let (mut score, hits) = found.map_or((0, Vec::new()), |m| (m.score, m.hits));
            if by_pick {
                score += LEARNED;
            }
            if by_key {
                score += KEY;
            }
            let boost = (120.0 * (1.0 + history.frecency(&app.id, now)).log2()).min(MAX_BOOST);
            scored.push((score + boost as i64, index, hits));
        }
        // Equal scores keep the order the apps were listed in (by name).
        scored.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
        scored
            .into_iter()
            .take(limit)
            .map(|(_, i, h)| (i, h))
            .collect()
    }
}

/// What the empty query shows: the apps opened lately, then the apps that
/// have a letter, then nothing more.
pub fn recent(
    apps: &[AppEntry],
    history: &History,
    keyed: impl Fn(&AppEntry) -> Option<char>,
    limit: usize,
) -> Vec<usize> {
    let mut out: Vec<usize> = history
        .recent()
        .into_iter()
        .filter_map(|id| apps.iter().position(|a| a.id == id))
        .collect();
    let mut keyed_apps: Vec<(char, usize)> = apps
        .iter()
        .enumerate()
        .filter_map(|(i, a)| keyed(a).map(|k| (k, i)))
        .collect();
    keyed_apps.sort();
    for (_, i) in keyed_apps {
        if !out.contains(&i) {
            out.push(i);
        }
    }
    out.truncate(limit);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    fn apps() -> Vec<AppEntry> {
        [
            ("com.apple.Safari", "Safari"),
            ("com.apple.Notes", "Notes"),
            ("com.mitchellh.ghostty", "Ghostty"),
            ("app.zen", "Zen Browser"),
            ("org.zed", "Zed"),
            ("com.microsoft.VSCode", "Visual Studio Code"),
            ("com.spotify", "Spotify"),
            ("ru.keepcoder.Telegram", "Telegram"),
            ("md.obsidian", "Obsidian"),
            ("com.apple.TextEdit", "TextEdit"),
            ("com.apple.Terminal", "Terminal"),
        ]
        .iter()
        .map(|(id, name)| AppEntry::new(id, name, &format!("/Applications/{name}.app")))
        .collect()
    }

    fn keys() -> BTreeMap<&'static str, char> {
        BTreeMap::from([("Ghostty", 't'), ("Zen Browser", 'w'), ("Telegram", 'g')])
    }

    fn names(apps: &[AppEntry], ranked: &[(usize, Vec<u32>)]) -> Vec<String> {
        ranked.iter().map(|(i, _)| apps[*i].name.clone()).collect()
    }

    fn rank(query: &str, history: &History, now: u64) -> Vec<String> {
        let apps = apps();
        let keys = keys();
        let ranked = Ranker::new().apps(
            &apps,
            query,
            history,
            |a| keys.get(a.name.as_str()).copied(),
            now,
            9,
        );
        names(&apps, &ranked)
    }

    #[test]
    fn the_letter_key_wins() {
        let out = rank("t", &History::default(), 0);
        assert_eq!(out[0], "Ghostty", "{out:?}");
        // Without the key it would be a plain prefix match.
        assert!(out.contains(&"Telegram".to_string()));
    }

    #[test]
    fn a_learned_choice_beats_text_for_that_exact_query() {
        let mut h = History::default();
        let before = rank("te", &h, 100);
        assert_ne!(before[0], "TextEdit");
        h.record("te", "com.apple.TextEdit", 100);
        assert_eq!(rank("te", &h, 100)[0], "TextEdit");
        assert_eq!(rank("ter", &h, 100)[0], "Terminal", "only the exact query");
    }

    #[test]
    fn a_learned_choice_may_have_no_text_in_common() {
        let mut h = History::default();
        h.record("browser", "app.zen", 5);
        assert_eq!(rank("browser", &h, 5)[0], "Zen Browser");
    }

    #[test]
    fn prefix_beats_initials_beats_fuzzy() {
        let apps = vec![
            AppEntry::new("a", "Vsc Tools", "/a"),
            AppEntry::new("b", "Visual Studio Code", "/b"),
            AppEntry::new("c", "Vivid Super Calendar App", "/c"),
            AppEntry::new("d", "Avast Software Client", "/d"),
        ];
        let ranked = Ranker::new().apps(&apps, "vsc", &History::default(), |_| None, 0, 9);
        assert_eq!(names(&apps, &ranked)[0], "Vsc Tools", "prefix first");
        let rest: Vec<_> = names(&apps, &ranked)[1..].to_vec();
        assert_eq!(rest[0], "Visual Studio Code", "{rest:?}");
        assert!(rest.contains(&"Vivid Super Calendar App".to_string()));
        // "Avast Software Client" has v, s, c in order but not at word starts.
        assert!(rest.iter().position(|n| n == "Avast Software Client") > Some(0));
    }

    #[test]
    fn vsc_finds_visual_studio_code_and_highlights_the_initials() {
        let apps = apps();
        let ranked = Ranker::new().apps(&apps, "vsc", &History::default(), |_| None, 0, 9);
        assert_eq!(apps[ranked[0].0].name, "Visual Studio Code");
        assert_eq!(ranked[0].1, [0, 7, 14]);
    }

    #[test]
    fn prefix_highlights_the_typed_letters() {
        let apps = apps();
        let ranked = Ranker::new().apps(&apps, "zen", &History::default(), |_| None, 0, 9);
        assert_eq!(apps[ranked[0].0].name, "Zen Browser");
        assert_eq!(ranked[0].1, [0, 1, 2]);
    }

    #[test]
    fn a_later_word_counts_as_a_prefix_of_that_word() {
        let apps = apps();
        let ranked = Ranker::new().apps(&apps, "stud", &History::default(), |_| None, 0, 9);
        assert_eq!(apps[ranked[0].0].name, "Visual Studio Code");
        assert_eq!(ranked[0].1, [7, 8, 9, 10]);
    }

    #[test]
    fn fuzzy_finds_scattered_letters() {
        let out = rank("tlgrm", &History::default(), 0);
        assert_eq!(out, ["Telegram"]);
    }

    #[test]
    fn use_orders_within_a_step_but_cannot_beat_a_better_match() {
        let mut h = History::default();
        // Terminal and TextEdit both start with "te"; Telegram too.
        assert_ne!(rank("te", &h, 1000)[0], "Terminal");
        for _ in 0..30 {
            h.record("", "com.apple.Terminal", 1000);
        }
        let boosted = rank("te", &h, 1000);
        assert_eq!(boosted[0], "Terminal");
        // A very heavily used app that only matches fuzzily stays below a prefix match.
        let apps = vec![
            AppEntry::new("safari", "Safari", "/s"),
            AppEntry::new("arc", "Arc", "/a"),
        ];
        for _ in 0..5000 {
            h.record("", "safari", 1000);
        }
        let ranked = Ranker::new().apps(&apps, "ar", &h, |_| None, 1000, 9);
        assert_eq!(apps[ranked[0].0].name, "Arc");
        assert_eq!(ranked.len(), 2);
    }

    #[test]
    fn the_order_depends_only_on_the_query_and_the_history() {
        let h = History::default();
        let first = rank("te", &h, 0);
        assert_eq!(first, rank("te", &h, 0));
        // Typing more never promotes a worse prefix match above a better one.
        assert_eq!(rank("tex", &h, 0)[0], "TextEdit");
    }

    #[test]
    fn aliases_find_an_app_but_never_highlight() {
        let mut apps = apps();
        apps[5].aliases.push("Code".into());
        let ranked = Ranker::new().apps(&apps, "code", &History::default(), |_| None, 0, 9);
        assert_eq!(apps[ranked[0].0].name, "Visual Studio Code");
        assert!(
            ranked[0].1.len() == 4,
            "the word 'Code' still matches the name"
        );
        let ranked = Ranker::new().apps(&apps, "cod", &History::default(), |_| None, 0, 9);
        assert!(!ranked.is_empty());
    }

    #[test]
    fn recent_lists_used_apps_then_keyed_ones() {
        let apps = apps();
        let keys = keys();
        let mut h = History::default();
        h.record("", "com.spotify", 10);
        h.record("", "app.zen", 20);
        h.record("", "gone.app", 30);
        let out = recent(&apps, &h, |a| keys.get(a.name.as_str()).copied(), 9);
        let out: Vec<_> = out.iter().map(|i| apps[*i].name.as_str()).collect();
        assert_eq!(out, ["Zen Browser", "Spotify", "Telegram", "Ghostty"]);
    }

    #[test]
    fn unicode_names_do_not_panic() {
        let apps = vec![AppEntry::new("a", "Äpfel Ünïcode", "/a")];
        let ranked = Ranker::new().apps(&apps, "ünï", &History::default(), |_| None, 0, 9);
        assert_eq!(ranked[0].1, [6, 7, 8]);
    }
}
