//! Web search engines: `?nix ripgrep`, `?gh telmo`.

use crate::config::Config;
use ratatui::style::Color;
use telmo_kit::theme;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Engine {
    /// What follows the `?`.
    pub key: String,
    pub label: String,
    /// `%s` stands for the search words.
    pub url: String,
    pub glyph: &'static str,
    pub color: Color,
}

const DEFAULTS: [(&str, &str); 4] = [
    ("nix", "https://search.nixos.org/packages?query=%s"),
    ("gh", "https://github.com/search?q=%s"),
    ("yt", "https://www.youtube.com/results?search_query=%s"),
    ("g", "https://www.google.com/search?q=%s"),
];

/// Name, glyph and color of the engines we know; others show their host.
fn known(key: &str) -> Option<(&'static str, &'static str, Color)> {
    Some(match key {
        "nix" => ("Nix packages", "\u{f313}", theme::CYAN),
        "gh" => ("GitHub", "\u{f09b}", theme::FG),
        "yt" => ("YouTube", "\u{f16a}", theme::RED),
        "g" => ("Google", "\u{f1a0}", theme::BLUE),
        "maps" => ("Maps", "\u{f041}", theme::GREEN),
        _ => return None,
    })
}

fn host(url: &str) -> String {
    let rest = url.split_once("://").map_or(url, |(_, rest)| rest);
    let host = rest.split(['/', '?']).next().unwrap_or(rest);
    host.strip_prefix("www.").unwrap_or(host).to_string()
}

/// The configured engines, the default one first, the rest by name. Without
/// a `search` setting: nix, gh, yt, g.
pub fn engines(config: &Config) -> Vec<Engine> {
    let mut list: Vec<(String, String)> = if config.search.is_empty() {
        DEFAULTS
            .iter()
            .map(|(k, u)| (k.to_string(), u.to_string()))
            .collect()
    } else {
        config
            .search
            .iter()
            .map(|(k, u)| (k.clone(), u.clone()))
            .collect()
    };
    let first = config
        .default_search
        .as_deref()
        .filter(|d| list.iter().any(|(k, _)| k == d))
        .unwrap_or(if list.iter().any(|(k, _)| k == "nix") {
            "nix"
        } else {
            list.first().map_or("", |(k, _)| k)
        })
        .to_string();
    if let Some(at) = list.iter().position(|(k, _)| *k == first) {
        let engine = list.remove(at);
        list.insert(0, engine);
    }
    list.into_iter()
        .map(|(key, url)| {
            let (label, glyph, color) = match known(&key) {
                Some((label, glyph, color)) => (label.to_string(), glyph, color),
                None => (host(&url), "\u{f0ac}", theme::DIM),
            };
            Engine {
                key,
                label,
                url,
                glyph,
                color,
            }
        })
        .collect()
}

/// Splits what follows the `?` into the engine it names (an index) and the
/// search words. Without a name the words are everything.
pub fn split<'a>(rest: &'a str, engines: &[Engine]) -> (Option<usize>, &'a str) {
    let (word, words) = rest.split_once(' ').unwrap_or((rest, ""));
    match engines.iter().position(|e| e.key == word) {
        Some(at) => (Some(at), words.trim()),
        None => (None, rest.trim()),
    }
}

/// Percent-encodes everything but the characters URLs leave alone.
pub fn encode(text: &str) -> String {
    let mut out = String::new();
    for byte in text.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                out.push(byte as char)
            }
            _ => out.push_str(&format!("%{byte:02X}")),
        }
    }
    out
}

/// The page for a search, or `None` when there is nothing to search for.
pub fn url_for(engine: &Engine, words: &str) -> Option<String> {
    let words = words.trim();
    if words.is_empty() {
        return None;
    }
    Some(engine.url.replace("%s", &encode(words)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_start_with_nix() {
        let list = engines(&Config::default());
        let keys: Vec<_> = list.iter().map(|e| e.key.as_str()).collect();
        assert_eq!(keys, ["nix", "gh", "yt", "g"]);
    }

    #[test]
    fn the_configured_default_goes_first_and_hosts_name_unknown_engines() {
        let mut config = Config::default();
        config
            .search
            .insert("a".into(), "https://www.example.org/s?q=%s".into());
        config
            .search
            .insert("gh".into(), "https://github.com/search?q=%s".into());
        config.default_search = Some("gh".into());
        let list = engines(&config);
        assert_eq!(list[0].key, "gh");
        assert_eq!(list[1].label, "example.org");
        config.default_search = None;
        assert_eq!(engines(&config)[0].key, "a", "no nix: the first by name");
    }

    #[test]
    fn a_named_engine_is_split_off() {
        let list = engines(&Config::default());
        assert_eq!(split("nix ripgrep", &list), (Some(0), "ripgrep"));
        assert_eq!(split("gh telmo  ", &list), (Some(1), "telmo"));
        assert_eq!(split("gh", &list), (Some(1), ""));
        assert_eq!(split("ripgrep fast", &list), (None, "ripgrep fast"));
    }

    #[test]
    fn urls_are_built_and_encoded() {
        let list = engines(&Config::default());
        assert_eq!(
            url_for(&list[0], "ripgrep").unwrap(),
            "https://search.nixos.org/packages?query=ripgrep"
        );
        assert_eq!(
            url_for(&list[3], "c++ & más").unwrap(),
            "https://www.google.com/search?q=c%2B%2B%20%26%20m%C3%A1s"
        );
        assert_eq!(url_for(&list[0], "  "), None);
    }
}
