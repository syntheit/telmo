//! What a piece of copied text is: a link, a color, code or plain text.

use crate::color::Rgb;
use crate::model::Kind;

pub fn kind_of(text: &str) -> Kind {
    let trimmed = text.trim();
    if is_link(trimmed) {
        Kind::Link
    } else if Rgb::parse(trimmed).is_some() {
        Kind::Color
    } else if looks_like_code(trimmed) {
        Kind::Code
    } else {
        Kind::Text
    }
}

/// A single URL: a known scheme, or a bare domain followed by a path.
pub fn is_link(text: &str) -> bool {
    if text.is_empty() || text.contains(char::is_whitespace) {
        return false;
    }
    let lower = text.to_ascii_lowercase();
    for scheme in ["http://", "https://", "ftp://"] {
        if let Some(rest) = lower.strip_prefix(scheme) {
            return !rest.is_empty();
        }
    }
    if let Some(rest) = lower.strip_prefix("mailto:") {
        return rest.contains('@');
    }
    bare_domain_with_path(&lower)
}

fn bare_domain_with_path(text: &str) -> bool {
    let Some((host, _path)) = text.split_once('/') else {
        return false;
    };
    let host = host.split(':').next().unwrap_or(host);
    let labels: Vec<&str> = host.split('.').collect();
    let tld_ok = labels
        .last()
        .is_some_and(|t| t.len() >= 2 && t.chars().all(|c| c.is_ascii_alphabetic()));
    labels.len() >= 2
        && tld_ok
        && labels
            .iter()
            .all(|l| !l.is_empty() && l.chars().all(|c| c.is_ascii_alphanumeric() || c == '-'))
}

/// Several lines, and a good share of them end like code does.
fn looks_like_code(text: &str) -> bool {
    let lines: Vec<&str> = text.lines().filter(|l| !l.trim().is_empty()).collect();
    if lines.len() < 2 {
        return false;
    }
    let codey = lines.iter().filter(|l| line_looks_like_code(l)).count();
    codey >= 2 && codey * 2 >= lines.len()
}

fn line_looks_like_code(line: &str) -> bool {
    let trimmed = line.trim_end();
    let first = line.trim_start();
    let block = trimmed.ends_with([';', '{', '}']);
    let call = trimmed.ends_with([')', ',']) && first.contains('(');
    let keyword = [
        "fn ",
        "def ",
        "class ",
        "import ",
        "function ",
        "const ",
        "let ",
        "#include",
    ]
    .iter()
    .any(|k| first.starts_with(k));
    block || call || keyword
}

/// The host and the rest of a link, for the details line.
pub fn split_link(url: &str) -> (String, String) {
    let url = url.trim();
    if let Some(address) = url.strip_prefix("mailto:") {
        return (address.to_string(), String::new());
    }
    let rest = url.split_once("://").map_or(url, |(_, rest)| rest);
    let end = rest.find(['/', '?', '#']).unwrap_or(rest.len());
    let host = rest[..end].rsplit('@').next().unwrap_or(&rest[..end]);
    let path = &rest[end..];
    (host.to_string(), path.to_string())
}

/// Characters, words and lines.
pub fn counts(text: &str) -> (u32, u32, u32) {
    (
        text.chars().count() as u32,
        text.split_whitespace().count() as u32,
        text.lines().count().max(1) as u32,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn links() {
        for text in [
            "https://github.com/syntheit/telmo/pull/12",
            "HTTP://example.com",
            "ftp://files.example.com/a",
            "mailto:daniel@matv.io",
            "news.ycombinator.com/item?id=1",
            "example.com/a",
        ] {
            assert_eq!(kind_of(text), Kind::Link, "{text}");
        }
    }

    #[test]
    fn not_links() {
        for text in [
            "example.com",
            "see https://example.com now",
            "daniel@matv.io",
            "a/b",
            "file.txt/x y",
            "https://",
        ] {
            assert_ne!(kind_of(text), Kind::Link, "{text}");
        }
    }

    #[test]
    fn colors() {
        for text in ["#7aa2f7", "rgb(1, 2, 3)", " hsl(220, 89%, 72%)\n"] {
            assert_eq!(kind_of(text), Kind::Color, "{text}");
        }
        assert_eq!(kind_of("#nope"), Kind::Text);
    }

    #[test]
    fn code_needs_several_lines() {
        assert_eq!(kind_of("fn main() {\n    println!(\"hi\");\n}"), Kind::Code);
        assert_eq!(kind_of("fn main() { println!(\"hi\"); }"), Kind::Text);
        assert_eq!(kind_of("Meeting moved to 3:30\nsame room"), Kind::Text);
        assert_eq!(
            kind_of("sudo darwin-rebuild switch\n  'https://x.org'"),
            Kind::Text
        );
    }

    #[test]
    fn splits_links() {
        assert_eq!(
            split_link("https://github.com/syntheit/telmo/pull/12"),
            ("github.com".into(), "/syntheit/telmo/pull/12".into())
        );
        assert_eq!(
            split_link("example.com/a?b=1"),
            ("example.com".into(), "/a?b=1".into())
        );
        assert_eq!(
            split_link("https://u:p@host.org"),
            ("host.org".into(), String::new())
        );
    }

    #[test]
    fn counts_text() {
        assert_eq!(counts("a b\n  c"), (7, 3, 2));
        assert_eq!(counts("x"), (1, 1, 1));
    }
}
