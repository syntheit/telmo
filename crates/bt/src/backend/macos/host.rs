//! The Telmo host app keeps the no-auto-connect list and drops devices on it
//! that connect without us asking. We reach it over its socket.

use serde::Deserialize;
use std::time::Duration;

const LIMIT: Duration = Duration::from_secs(2);

#[derive(Debug, Default, Deserialize)]
struct GuardStatus {
    #[serde(default)]
    no_auto_connect: Vec<String>,
}

fn command(line: &str) -> Result<String, String> {
    telmo_kit::host::command(line, LIMIT, LIMIT)
}

/// "AA:BB:CC:DD:EE:FF", any case.
pub fn valid_address(address: &str) -> bool {
    let parts: Vec<&str> = address.split(':').collect();
    parts.len() == 6
        && parts
            .iter()
            .all(|p| p.len() == 2 && p.chars().all(|c| c.is_ascii_hexdigit()))
}

fn address_line(verb: &str, address: &str, suffix: &str) -> Result<String, String> {
    if !valid_address(address) {
        return Err(format!("{address} is not a Bluetooth address."));
    }
    Ok(format!("{verb} {address}{suffix}"))
}

/// Addresses that must not connect on their own. None when the host isn't running.
pub fn no_auto_connect() -> Option<Vec<String>> {
    let status: GuardStatus = serde_json::from_str(&command("bt-guard-status").ok()?).ok()?;
    Some(
        status
            .no_auto_connect
            .iter()
            .map(|a| a.to_uppercase())
            .collect(),
    )
}

/// Tell the host the next connection is ours. Best effort: without a host nothing drops it.
pub fn allow(address: &str) {
    if let Ok(line) = address_line("bt-allow", address, "") {
        let _ = command(&line);
    }
}

pub fn set_auto_connect(address: &str, on: bool) -> Result<(), String> {
    let suffix = if on { " off" } else { " on" };
    command(&address_line("bt-no-autoconnect", address, suffix)?).map(|_| ())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validates_addresses() {
        assert!(valid_address("4C:87:5D:1A:0F:22"));
        assert!(valid_address("4c:87:5d:1a:0f:22"));
        for bad in [
            "",
            "4C:87:5D:1A:0F",
            "4C-87-5D-1A-0F-22",
            "4C:87:5D:1A:0F:2G",
            "4C:87:5D:1A:0F:22 x",
        ] {
            assert!(!valid_address(bad), "{bad}");
        }
    }

    #[test]
    fn auto_connect_off_means_listed() {
        assert_eq!(
            address_line("bt-no-autoconnect", "4C:87:5D:1A:0F:22", " on").as_deref(),
            Ok("bt-no-autoconnect 4C:87:5D:1A:0F:22 on")
        );
        assert!(address_line("bt-allow", "x\nbt-allow y", "").is_err());
    }
}
