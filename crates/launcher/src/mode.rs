//! The first character picks the mode.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode<'a> {
    /// Nothing typed: the apps used lately.
    Recent,
    Apps(&'a str),
    /// A calculation. Typed with `=`, it shows only the result; started by a
    /// number, apps match too (`1password`).
    Math {
        expr: &'a str,
        apps: bool,
    },
    /// Everything after the `!`.
    Commands(&'a str),
    /// Everything after the `?`.
    Web(&'a str),
}

impl<'a> Mode<'a> {
    pub fn parse(query: &'a str) -> Self {
        let query = query.trim_start();
        let mut chars = query.chars();
        let Some(first) = chars.next() else {
            return Mode::Recent;
        };
        let rest = chars.as_str();
        match first {
            '!' => Mode::Commands(rest.trim()),
            '?' => Mode::Web(rest.trim_start()),
            '=' => Mode::Math {
                expr: rest.trim(),
                apps: false,
            },
            c if c.is_ascii_digit() => Mode::Math {
                expr: query.trim(),
                apps: true,
            },
            '(' | '-' | '+' | '.' if rest.starts_with(|n: char| n.is_ascii_digit() || n == '(') => {
                Mode::Math {
                    expr: query.trim(),
                    apps: true,
                }
            }
            _ => Mode::Apps(query.trim()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_first_character_decides() {
        assert_eq!(Mode::parse(""), Mode::Recent);
        assert_eq!(Mode::parse("  "), Mode::Recent);
        assert_eq!(Mode::parse("zen"), Mode::Apps("zen"));
        assert_eq!(Mode::parse("!sl"), Mode::Commands("sl"));
        assert_eq!(Mode::parse("!"), Mode::Commands(""));
        assert_eq!(Mode::parse("?nix ripgrep"), Mode::Web("nix ripgrep"));
        assert_eq!(
            Mode::parse("= 2+2"),
            Mode::Math {
                expr: "2+2",
                apps: false
            }
        );
        assert_eq!(
            Mode::parse("2 ft to cm"),
            Mode::Math {
                expr: "2 ft to cm",
                apps: true
            }
        );
        assert_eq!(
            Mode::parse("-3*4"),
            Mode::Math {
                expr: "-3*4",
                apps: true
            }
        );
        assert_eq!(Mode::parse("-x"), Mode::Apps("-x"));
    }
}
