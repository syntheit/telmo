//! Reading and changing auto-join in macOS's root-only known-networks file.
//! Everything goes through /usr/bin/defaults with a fixed argv, never a shell.

use crate::request::{key, ssid_of_key};
use std::collections::BTreeMap;
use std::process::{Command, Stdio};

const DEFAULTS: &str = "/usr/bin/defaults";
const DOMAIN: &str = "/Library/Preferences/com.apple.wifi.known-networks";

/// SSID -> whether macOS joins it automatically.
pub type AutoJoin = BTreeMap<String, bool>;

pub fn parse(xml: &[u8]) -> Result<AutoJoin, String> {
    let value = plist::Value::from_reader(std::io::Cursor::new(xml))
        .map_err(|e| format!("The known-networks file isn't readable: {e}."))?;
    let dict = value
        .as_dictionary()
        .ok_or("The known-networks file has an unexpected shape.")?;
    let mut networks = AutoJoin::new();
    for (name, entry) in dict {
        let (Some(ssid), Some(entry)) = (ssid_of_key(name), entry.as_dictionary()) else {
            continue;
        };
        let disabled = entry
            .get("AutoJoinDisabled")
            .and_then(plist::Value::as_boolean)
            .unwrap_or(false);
        networks.insert(ssid.to_string(), !disabled);
    }
    Ok(networks)
}

pub fn list() -> Result<AutoJoin, String> {
    let output = run(&["export", DOMAIN, "-"])?;
    parse(&output)
}

pub fn set(ssid: &str, on: bool) -> Result<(), String> {
    if !list()?.contains_key(ssid) {
        return Err("That network isn't saved.".to_string());
    }
    let disabled = if on { "false" } else { "true" };
    run(&[
        "write",
        DOMAIN,
        &key(ssid),
        "-dict-add",
        "AutoJoinDisabled",
        "-bool",
        disabled,
    ])
    .map(|_| ())
}

fn run(args: &[&str]) -> Result<Vec<u8>, String> {
    let output = Command::new(DEFAULTS)
        .args(args)
        .env_clear()
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .map_err(|e| format!("Couldn't run defaults: {e}."))?;
    if output.status.success() {
        Ok(output.stdout)
    } else {
        Err("defaults failed to read or write the known networks.".to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
<key>wifi.network.ssid.Home Net</key><dict><key>AutoJoinDisabled</key><true/><key>Secret</key><string>x</string></dict>
<key>wifi.network.ssid.Cafe</key><dict><key>AddedAt</key><string>today</string></dict>
<key>wifi.network.ssid.Odd</key><string>not a dict</string>
<key>unrelated</key><dict/>
</dict></plist>"#;

    #[test]
    fn maps_ssids_to_auto_join_only() {
        let map = parse(SAMPLE.as_bytes()).unwrap();
        let expected: AutoJoin =
            [("Home Net".to_string(), false), ("Cafe".to_string(), true)].into();
        assert_eq!(map, expected);
    }

    #[test]
    fn rejects_garbage() {
        assert!(parse(b"nope").is_err());
        let array = br#"<?xml version="1.0"?><plist version="1.0"><array/></plist>"#;
        assert!(parse(array).is_err());
    }
}
