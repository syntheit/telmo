//! Search: case-insensitive, over what the list knows about an item.

use crate::model::Entry;

pub fn matches(entry: &Entry, query: &str) -> bool {
    let query = query.trim();
    if query.is_empty() {
        return true;
    }
    let query = query.to_lowercase();
    entry.preview.to_lowercase().contains(&query)
        || entry
            .files
            .iter()
            .any(|f| f.to_lowercase().contains(&query))
        || entry.source.to_lowercase().contains(&query)
}

/// Char ranges of `text` that match `query`, for highlighting.
pub fn ranges(text: &str, query: &str) -> Vec<std::ops::Range<usize>> {
    let query: Vec<char> = query.trim().chars().map(fold).collect();
    if query.is_empty() {
        return Vec::new();
    }
    let chars: Vec<char> = text.chars().map(fold).collect();
    let mut out = Vec::new();
    let mut at = 0;
    while at + query.len() <= chars.len() {
        if chars[at..at + query.len()] == query[..] {
            out.push(at..at + query.len());
            at += query.len();
        } else {
            at += 1;
        }
    }
    out
}

fn fold(c: char) -> char {
    c.to_lowercase().next().unwrap_or(c)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(preview: &str) -> Entry {
        Entry {
            preview: preview.into(),
            ..Entry::default()
        }
    }

    #[test]
    fn filters_ignoring_case() {
        assert!(matches(&entry("Darwin-Rebuild in ~/NIX"), "nix"));
        assert!(!matches(&entry("hello"), "nix"));
        assert!(matches(&entry("anything"), "  "));
    }

    #[test]
    fn matches_files_and_source() {
        let e = Entry {
            preview: "a.pdf".into(),
            files: vec!["/Users/me/Downloads/a.pdf".into()],
            source: "Finder".into(),
            ..Entry::default()
        };
        assert!(matches(&e, "downloads"));
        assert!(matches(&e, "finder"));
    }

    #[test]
    fn finds_ranges() {
        assert_eq!(ranges("nix and NIX", "nix"), vec![0..3, 8..11]);
        assert!(ranges("abc", "").is_empty());
        assert!(ranges("abc", "z").is_empty());
    }
}
