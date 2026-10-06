//! The Telmo host app. Only the signed app holds the Location grant that makes
//! CoreWLAN reveal network names, so inside the popup it does the scanning and
//! joining and we talk to it over its socket (never through argv: passwords).

use super::wifi::Entry;
use crate::model::{Band, Security};
use serde::Deserialize;
use std::io::{Read, Write};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::sync::{Mutex, PoisonError};
use std::time::Duration;

pub const QUICK: Duration = Duration::from_secs(5);
pub const SCAN: Duration = Duration::from_secs(15);
pub const JOIN: Duration = Duration::from_secs(40);
/// The host gives Touch ID / the keychain 60 s.
pub const REVEAL: Duration = Duration::from_secs(70);

pub fn running_in_host() -> bool {
    std::env::var_os("TELMO_HOST").is_some_and(|v| v == "1")
}

fn socket_path() -> Result<PathBuf, String> {
    if let Some(path) = std::env::var_os("TELMO_SOCKET") {
        return Ok(path.into());
    }
    let home = std::env::var_os("HOME").ok_or("HOME is not set.")?;
    Ok(PathBuf::from(home).join("Library/Application Support/Telmo/host.sock"))
}

/// Send one command and read the reply until the host closes the connection.
pub fn command(command: &str, limit: Duration) -> Result<String, String> {
    let unreachable = |e: std::io::Error| format!("Couldn't reach the Telmo host: {e}.");
    let mut stream = UnixStream::connect(socket_path()?).map_err(unreachable)?;
    stream.set_read_timeout(Some(limit)).map_err(unreachable)?;
    stream.set_write_timeout(Some(QUICK)).map_err(unreachable)?;
    stream
        .write_all(format!("{command}\n").as_bytes())
        .map_err(unreachable)?;
    let mut reply = String::new();
    stream.read_to_string(&mut reply).map_err(unreachable)?;
    let reply = reply.trim_end_matches('\n');
    match reply.strip_prefix("error ") {
        Some(message) => Err(message.to_string()),
        None => Ok(reply.to_string()),
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct Listing {
    pub current: Option<String>,
    pub networks: Vec<HostNetwork>,
    #[serde(default)]
    pub saved: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct HostNetwork {
    pub ssid: String,
    pub rssi: isize,
    pub security: String,
    pub band: String,
    pub channel: isize,
}

impl Listing {
    pub fn parse(json: &str) -> Result<Listing, String> {
        serde_json::from_str(json)
            .map_err(|e| format!("The Telmo host sent an unreadable list: {e}."))
    }

    pub fn entries(&self) -> Vec<Entry> {
        self.networks
            .iter()
            .map(|n| Entry {
                connected: false,
                ssid: Some(n.ssid.clone()),
                bssid: None,
                rssi: n.rssi,
                channel: n.channel,
                band: match n.band.as_str() {
                    "2" => Some(Band::G2),
                    "5" => Some(Band::G5),
                    "6" => Some(Band::G6),
                    _ => None,
                },
                security: match n.security.as_str() {
                    "open" => Security::Open,
                    "wep" => Security::Wep,
                    "wpa3" => Security::Wpa3Personal,
                    "enterprise" => Security::Enterprise,
                    _ => Security::Personal,
                },
            })
            .collect()
    }
}

/// The last list the host sent, if it had names in it.
static LATEST: Mutex<Option<Listing>> = Mutex::new(None);

pub fn latest() -> Option<Listing> {
    LATEST
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .clone()
}

/// Ask the host for networks (`wifi-scan` or `wifi-cached`) and remember them.
/// A host without Location returns no names; that counts as no list.
pub fn fetch(host_command: &str, limit: Duration) -> Result<(), String> {
    let listing = Listing::parse(&command(host_command, limit)?)?;
    let usable = !listing.networks.is_empty();
    *LATEST.lock().unwrap_or_else(PoisonError::into_inner) = usable.then_some(listing);
    if usable {
        Ok(())
    } else {
        Err("The Telmo host sees no network names yet.".to_string())
    }
}

/// Join through the host if it listed this network.
pub fn join(ssid: &str, password: Option<&str>) -> Option<Result<String, String>> {
    latest()?.networks.iter().find(|n| n.ssid == ssid)?;
    let line = format!("wifi-join {ssid}\t{}", password.unwrap_or(""));
    Some(command(&line, JOIN).map(|_| format!("Joined {ssid}.")))
}

/// The saved password, read by the host (Touch ID through sudo).
pub fn reveal_password(ssid: &str) -> Result<String, String> {
    let line = password_line(ssid)?;
    let reply = command(&line, REVEAL)?;
    // "ok " keeps a password that starts with "error " from looking like a failure.
    Ok(reply.strip_prefix("ok ").unwrap_or(&reply).to_string())
}

fn password_line(ssid: &str) -> Result<String, String> {
    if ssid.contains('\n') {
        return Err("This network name can't be looked up.".to_string());
    }
    Ok(format!("wifi-password {ssid}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn password_line_is_one_line() {
        assert_eq!(
            password_line("My Wi-Fi").as_deref(),
            Ok("wifi-password My Wi-Fi")
        );
        assert!(password_line("a\nb").is_err());
    }

    const JSON: &str = r#"{"current":"Home","networks":[
        {"ssid":"Home","rssi":-52,"security":"wpa3","band":"5","channel":149},
        {"ssid":"Cafe","rssi":-80,"security":"open","band":"2","channel":6}],
        "saved":["Home"]}"#;

    #[test]
    fn maps_host_json_to_entries() {
        let listing = Listing::parse(JSON).expect("parse");
        assert_eq!(listing.current.as_deref(), Some("Home"));
        let entries = listing.entries();
        assert_eq!(entries[0].security, Security::Wpa3Personal);
        assert_eq!(entries[0].band, Some(Band::G5));
        assert_eq!(entries[1].security, Security::Open);
        assert_eq!(entries[1].channel, 6);
    }

    #[test]
    fn current_may_be_null_and_saved_missing() {
        let listing = Listing::parse(r#"{"current":null,"networks":[]}"#).expect("parse");
        assert!(listing.current.is_none() && listing.saved.is_empty());
    }

    #[test]
    fn rejects_garbage() {
        assert!(Listing::parse("nope").is_err());
    }
}
