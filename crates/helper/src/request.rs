//! The helper's entire protocol: one request line in, one reply out.

/// A request line is at most this many bytes, newline included.
pub const MAX_LINE: usize = 256;
const MAX_SSID: usize = 32;
const KEY_PREFIX: &str = "wifi.network.ssid.";

#[derive(Debug, PartialEq, Eq)]
pub enum Request {
    AutojoinList,
    AutojoinSet { ssid: String, on: bool },
}

pub fn parse(line: &str) -> Result<Request, String> {
    if line == "autojoin-list" {
        return Ok(Request::AutojoinList);
    }
    if let Some(args) = line.strip_prefix("autojoin-set ") {
        // The SSID may contain spaces, so the state is whatever follows the last one.
        let (ssid, state) = args
            .rsplit_once(' ')
            .ok_or("Usage: autojoin-set <ssid> <on|off>.")?;
        let on = match state {
            "on" => true,
            "off" => false,
            _ => return Err("The state must be on or off.".to_string()),
        };
        validate_ssid(ssid)?;
        return Ok(Request::AutojoinSet {
            ssid: ssid.to_string(),
            on,
        });
    }
    Err("Unknown command.".to_string())
}

pub fn validate_ssid(ssid: &str) -> Result<(), String> {
    if ssid.is_empty() || ssid.len() > MAX_SSID {
        return Err("A network name is 1 to 32 bytes.".to_string());
    }
    if ssid.chars().any(char::is_control) {
        return Err("A network name can't contain control characters.".to_string());
    }
    Ok(())
}

/// The top-level key macOS stores a saved network under.
pub fn key(ssid: &str) -> String {
    format!("{KEY_PREFIX}{ssid}")
}

pub fn ssid_of_key(key: &str) -> Option<&str> {
    key.strip_prefix(KEY_PREFIX)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_the_two_commands() {
        assert_eq!(parse("autojoin-list"), Ok(Request::AutojoinList));
        assert_eq!(
            parse("autojoin-set Home Net off"),
            Ok(Request::AutojoinSet {
                ssid: "Home Net".into(),
                on: false
            })
        );
        assert_eq!(
            parse("autojoin-set cafe on"),
            Ok(Request::AutojoinSet {
                ssid: "cafe".into(),
                on: true
            })
        );
    }

    #[test]
    fn refuses_unknown_commands() {
        for line in [
            "",
            "ls",
            "autojoin-list x",
            "autojoin-listing",
            "AUTOJOIN-LIST",
            "defaults write",
            "autojoin-set",
        ] {
            assert!(parse(line).is_err(), "{line:?}");
        }
    }

    #[test]
    fn refuses_bad_states_and_ssids() {
        assert!(parse("autojoin-set cafe maybe").is_err());
        assert!(parse("autojoin-set cafe").is_err());
        assert!(parse("autojoin-set  on").is_err());
        assert!(parse(&format!("autojoin-set {} on", "a".repeat(33))).is_err());
        assert!(parse("autojoin-set a\u{7}b on").is_err());
        assert!(parse("autojoin-set a\rb on").is_err());
        assert!(parse("autojoin-set a\nb on").is_err());
        assert!(parse(&format!("autojoin-set {} on", "a".repeat(32))).is_ok());
    }

    #[test]
    fn ssid_limit_counts_bytes() {
        assert!(validate_ssid(&"é".repeat(16)).is_ok());
        assert!(validate_ssid(&"é".repeat(17)).is_err());
    }

    #[test]
    fn builds_and_reads_keys() {
        assert_eq!(key("Home"), "wifi.network.ssid.Home");
        assert_eq!(ssid_of_key("wifi.network.ssid.Home"), Some("Home"));
        assert_eq!(ssid_of_key("other.key"), None);
    }
}
