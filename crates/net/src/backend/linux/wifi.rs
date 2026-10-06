//! Wi-Fi: scanning, networks in range, joining, forgetting, passwords.

use super::nm::*;
use crate::model::{Band, Details, Network, Security, Wifi};
use futures_util::StreamExt;
use std::collections::HashMap;
use std::time::Duration;
use tokio::time::{sleep, timeout};
use zbus::Connection;
use zbus::zvariant::{ObjectPath, OwnedObjectPath, Value};

const JOIN_TIMEOUT: Duration = Duration::from_secs(30);
const SCAN_TIMEOUT: Duration = Duration::from_secs(10);
const SECURITY_SETTING: &str = "802-11-wireless-security";

// NM80211ApSecurityFlags
const FLAG_PRIVACY: u32 = 0x1;
const KEY_MGMT_PSK: u32 = 0x100;
const KEY_MGMT_802_1X: u32 = 0x200;
const KEY_MGMT_SAE: u32 = 0x400;

/// What we know about one access point.
#[derive(Debug, Clone, PartialEq)]
struct Ap {
    path: String,
    ssid: Option<String>,
    strength: u8,
    security: Security,
    frequency: u32,
}

/// The first managed Wi-Fi adapter.
pub async fn device(conn: &Connection) -> zbus::Result<Option<OwnedObjectPath>> {
    let nm = manager(conn).await?;
    for path in nm.get_devices().await? {
        let device = open!(DeviceProxy, conn, &path)?;
        if device.device_type().await? == DEVICE_WIFI && device.managed().await? {
            return Ok(Some(path));
        }
    }
    Ok(None)
}

/// SSID of the network a Wi-Fi device is connected to.
pub async fn connected_ssid(conn: &Connection, device: &OwnedObjectPath) -> Option<String> {
    let active = OwnedObjectPath::try_from(active_ap(conn, device).await?).ok()?;
    read_ap(conn, &active).await.ok()?.ssid
}

pub async fn read(conn: &Connection, scanning: bool) -> zbus::Result<Option<Wifi>> {
    let Some(path) = device(conn).await? else {
        return Ok(None);
    };
    let power = manager(conn).await?.wireless_enabled().await?;
    let saved: Vec<String> = saved_connections(conn)
        .await?
        .into_iter()
        .map(|(_, ssid)| ssid)
        .collect();
    let aps = if power {
        read_aps(conn, &path).await?
    } else {
        Vec::new()
    };
    let active = if power {
        active_ap(conn, &path).await
    } else {
        None
    };
    let networks = group_networks(&aps, active.as_deref(), &saved);
    let mut saved_elsewhere: Vec<String> = saved
        .into_iter()
        .filter(|s| !networks.iter().any(|n| n.ssid.as_deref() == Some(s)))
        .collect();
    saved_elsewhere.sort();
    saved_elsewhere.dedup();
    Ok(Some(Wifi {
        interface: path.to_string(),
        power,
        scanning,
        networks,
        saved_elsewhere,
        names_hidden: false,
    }))
}

async fn active_ap(conn: &Connection, device: &OwnedObjectPath) -> Option<String> {
    let wireless = open!(DeviceWirelessProxy, conn, device).ok()?;
    let active = wireless.active_access_point().await.ok()?;
    (!is_none(&active)).then(|| active.to_string())
}

async fn read_aps(conn: &Connection, device: &OwnedObjectPath) -> zbus::Result<Vec<Ap>> {
    let wireless = open!(DeviceWirelessProxy, conn, device)?;
    let mut aps = Vec::new();
    for path in wireless.get_all_access_points().await? {
        // An AP can vanish between listing and reading it.
        if let Ok(ap) = read_ap(conn, &path).await {
            aps.push(ap);
        }
    }
    Ok(aps)
}

async fn read_ap(conn: &Connection, path: &OwnedObjectPath) -> zbus::Result<Ap> {
    let ap = open!(AccessPointProxy, conn, path)?;
    let ssid = String::from_utf8_lossy(&ap.ssid().await?)
        .trim()
        .to_string();
    Ok(Ap {
        path: path.to_string(),
        ssid: (!ssid.is_empty()).then_some(ssid),
        strength: ap.strength().await?.min(100),
        security: security_of(
            ap.flags().await?,
            ap.wpa_flags().await?,
            ap.rsn_flags().await?,
        ),
        frequency: ap.frequency().await?,
    })
}

fn security_of(flags: u32, wpa: u32, rsn: u32) -> Security {
    let all = wpa | rsn;
    if all & KEY_MGMT_802_1X != 0 {
        Security::Enterprise
    } else if rsn & KEY_MGMT_SAE != 0 && all & KEY_MGMT_PSK == 0 {
        Security::Wpa3Personal
    } else if all & (KEY_MGMT_PSK | KEY_MGMT_SAE) != 0 {
        Security::Personal
    } else if flags & FLAG_PRIVACY != 0 {
        Security::Wep
    } else {
        Security::Open
    }
}

fn band_of(frequency: u32) -> Option<Band> {
    match frequency {
        2400..=2500 => Some(Band::G2),
        4900..=5900 => Some(Band::G5),
        5925..=7125 => Some(Band::G6),
        _ => None,
    }
}

fn channel_of(frequency: u32) -> Option<u32> {
    match band_of(frequency)? {
        Band::G2 if frequency == 2484 => Some(14),
        Band::G2 => Some((frequency - 2407) / 5),
        Band::G5 => Some((frequency - 5000) / 5),
        Band::G6 => Some((frequency - 5950) / 5),
    }
}

/// One entry per SSID (the strongest AP), hidden networks together, strongest first.
fn group_networks(aps: &[Ap], active: Option<&str>, saved: &[String]) -> Vec<Network> {
    let mut best: HashMap<Option<&str>, &Ap> = HashMap::new();
    for ap in aps {
        let slot = best.entry(ap.ssid.as_deref()).or_insert(ap);
        if ap.strength > slot.strength {
            *slot = ap;
        }
    }
    let mut networks: Vec<Network> = best
        .into_iter()
        .map(|(ssid, ap)| Network {
            id: ssid.unwrap_or("hidden").to_string(),
            ssid: ssid.map(str::to_string),
            strength: ap.strength,
            security: ap.security,
            band: band_of(ap.frequency),
            saved: ssid.is_some_and(|s| saved.iter().any(|x| x == s)),
            connected: aps
                .iter()
                .any(|a| a.ssid.as_deref() == ssid && Some(a.path.as_str()) == active),
        })
        .collect();
    networks.sort_by(|a, b| b.strength.cmp(&a.strength).then_with(|| a.id.cmp(&b.id)));
    networks
}

/// Saved Wi-Fi profiles as (profile path, SSID).
async fn saved_connections(conn: &Connection) -> zbus::Result<Vec<(OwnedObjectPath, String)>> {
    let settings = settings(conn).await?;
    let mut saved = Vec::new();
    for path in settings.list_connections().await? {
        let profile = open!(SettingsConnectionProxy, conn, &path)?;
        let Ok(profile_settings) = profile.get_settings().await else {
            continue;
        };
        if let Some(ssid) = wifi_ssid(&profile_settings) {
            saved.push((path, ssid));
        }
    }
    Ok(saved)
}

fn wifi_ssid(settings: &Settings) -> Option<String> {
    let kind = settings.get("connection")?.get("type")?;
    if kind.downcast_ref::<&str>().ok()? != "802-11-wireless" {
        return None;
    }
    let bytes = Vec::<u8>::try_from(
        settings
            .get("802-11-wireless")?
            .get("ssid")?
            .try_clone()
            .ok()?,
    )
    .ok()?;
    Some(String::from_utf8_lossy(&bytes).into_owned())
}

async fn profiles_for(conn: &Connection, ssid: &str) -> zbus::Result<Vec<OwnedObjectPath>> {
    Ok(saved_connections(conn)
        .await?
        .into_iter()
        .filter(|(_, s)| s == ssid)
        .map(|(path, _)| path)
        .collect())
}

/// Ask NM for a fresh scan. NM refuses when one just ran; that is fine.
pub async fn request_scan(conn: &Connection) -> Result<(), String> {
    let what = "scan for Wi-Fi networks";
    let path = device(conn)
        .await
        .map_err(|e| explain(what, &e))?
        .ok_or("This computer has no Wi-Fi adapter.")?;
    let wireless = open!(DeviceWirelessProxy, conn, &path).map_err(|e| explain(what, &e))?;
    match wireless.request_scan(HashMap::new()).await {
        Err(zbus::Error::MethodError(name, _, _)) if name.as_str().ends_with("NotAllowedOnNow") => {
            Ok(())
        }
        other => other.map_err(|e| explain(what, &e)),
    }
}

/// Request a scan and wait until NM reports it finished (or a timeout).
pub async fn scan(conn: &Connection) -> Result<(), String> {
    let path = device(conn)
        .await
        .map_err(|e| explain("scan for Wi-Fi networks", &e))?
        .ok_or("This computer has no Wi-Fi adapter.")?;
    let wireless = open!(DeviceWirelessProxy, conn, &path)
        .map_err(|e| explain("scan for Wi-Fi networks", &e))?;
    let before = wireless.last_scan().await.unwrap_or(0);
    request_scan(conn).await?;
    let finished = async {
        while wireless.last_scan().await.unwrap_or(before) == before {
            sleep(Duration::from_millis(400)).await;
        }
    };
    // A refused scan never advances LastScan, so a timeout is normal.
    let _ = timeout(SCAN_TIMEOUT, finished).await;
    Ok(())
}

pub async fn set_power(conn: &Connection, on: bool) -> Result<String, String> {
    let what = "change the Wi-Fi radio";
    let nm = manager(conn).await.map_err(|e| explain(what, &e))?;
    nm.set_wireless_enabled(on)
        .await
        .map_err(|e| explain(what, &e))?;
    Ok(if on { "Wi-Fi is on." } else { "Wi-Fi is off." }.to_string())
}

pub async fn forget(conn: &Connection, ssid: &str) -> Result<String, String> {
    let what = format!("forget {ssid}");
    let profiles = profiles_for(conn, ssid)
        .await
        .map_err(|e| explain(&what, &e))?;
    if profiles.is_empty() {
        return Err(format!("{ssid} isn't saved."));
    }
    for path in profiles {
        let profile =
            open!(SettingsConnectionProxy, conn, &path).map_err(|e| explain(&what, &e))?;
        profile.delete().await.map_err(|e| explain(&what, &e))?;
    }
    Ok(format!("Forgot {ssid}."))
}

pub async fn reveal_password(conn: &Connection, ssid: &str) -> Result<String, String> {
    let what = format!("read the password for {ssid}");
    let profiles = profiles_for(conn, ssid)
        .await
        .map_err(|e| explain(&what, &e))?;
    for path in profiles {
        let profile =
            open!(SettingsConnectionProxy, conn, &path).map_err(|e| explain(&what, &e))?;
        let secrets = profile
            .get_secrets(SECURITY_SETTING)
            .await
            .map_err(|e| explain(&what, &e))?;
        let psk = secrets
            .get(SECURITY_SETTING)
            .and_then(|s| s.get("psk"))
            .and_then(|v| v.downcast_ref::<&str>().ok());
        if let Some(psk) = psk {
            return Ok(psk.to_string());
        }
    }
    Err(format!("No password is stored for {ssid}."))
}

type NewSettings<'a> = HashMap<&'a str, HashMap<&'a str, Value<'a>>>;

/// Settings for a profile that doesn't exist yet.
fn new_profile<'a>(
    ssid: &'a str,
    security: Security,
    password: Option<&'a str>,
) -> Result<NewSettings<'a>, String> {
    let key_mgmt = match security {
        Security::Open => None,
        Security::Personal => Some("wpa-psk"),
        Security::Wpa3Personal => Some("sae"),
        Security::Wep => return Err(format!("{ssid} uses WEP, which isn't supported.")),
        Security::Enterprise => {
            return Err(format!(
                "{ssid} is a company network. Set it up once with nmcli or your desktop's settings."
            ));
        }
    };
    let mut settings: NewSettings = HashMap::new();
    settings.insert(
        "connection",
        HashMap::from([
            ("type", Value::from("802-11-wireless")),
            ("id", Value::from(ssid)),
        ]),
    );
    settings.insert(
        "802-11-wireless",
        HashMap::from([("ssid", Value::new(ssid.as_bytes().to_vec()))]),
    );
    if let Some(key_mgmt) = key_mgmt {
        let psk = password.ok_or(format!("{ssid} needs a password."))?;
        settings.insert(
            SECURITY_SETTING,
            HashMap::from([
                ("key-mgmt", Value::from(key_mgmt)),
                ("psk", Value::from(psk)),
            ]),
        );
    }
    Ok(settings)
}

enum Outcome {
    Connected,
    Failed(u32),
}

/// Join a network and wait until the device is connected or has given up.
pub async fn join(conn: &Connection, ssid: &str, password: Option<&str>) -> Result<String, String> {
    let what = format!("join {ssid}");
    let fail = |e: zbus::Error| explain(&what, &e);
    let path = device(conn)
        .await
        .map_err(fail)?
        .ok_or("This computer has no Wi-Fi adapter.")?;
    let ap = read_aps(conn, &path)
        .await
        .map_err(fail)?
        .into_iter()
        .filter(|a| a.ssid.as_deref() == Some(ssid))
        .max_by_key(|a| a.strength)
        .ok_or(format!("{ssid} is out of range."))?;
    let saved = profiles_for(conn, ssid).await.map_err(fail)?;
    let nm = manager(conn).await.map_err(fail)?;
    let device = open!(DeviceProxy, conn, &path).map_err(fail)?;
    let mut changes = device.receive_device_state_changed().await.map_err(fail)?;
    let ap_path = ObjectPath::try_from(ap.path.as_str()).map_err(|e| e.to_string())?;

    let added = match saved.first() {
        Some(profile) => {
            nm.activate_connection(profile, &path, &ap_path)
                .await
                .map_err(fail)?;
            None
        }
        None => {
            let settings = new_profile(ssid, ap.security, password)?;
            let (profile, _) = nm
                .add_and_activate_connection(settings, &path, &ap_path)
                .await
                .map_err(fail)?;
            Some(profile)
        }
    };

    let waiting = async {
        while let Some(change) = changes.next().await {
            let Ok(args) = change.args() else { continue };
            match args.new_state {
                STATE_ACTIVATED => return Outcome::Connected,
                STATE_FAILED => return Outcome::Failed(args.reason),
                _ => {}
            }
        }
        Outcome::Failed(0)
    };
    let outcome = timeout(JOIN_TIMEOUT, waiting).await;
    let reason = match outcome {
        Ok(Outcome::Connected) => return Ok(format!("Joined {ssid}.")),
        Ok(Outcome::Failed(reason)) => reason,
        Err(_) => {
            return Err(format!(
                "Timed out joining {ssid}. Check the password and signal."
            ));
        }
    };
    let message = failure_message(ssid, reason);
    if let Some(profile) = added
        && is_wrong_password(reason)
        && let Ok(profile) = open!(SettingsConnectionProxy, conn, &profile)
    {
        let _ = profile.delete().await;
    }
    Err(message)
}

// NMDeviceStateReason: NO_SECRETS and the supplicant failures.
fn is_wrong_password(reason: u32) -> bool {
    matches!(reason, 7..=11)
}

fn failure_message(ssid: &str, reason: u32) -> String {
    match reason {
        r if is_wrong_password(r) => format!("Wrong password for {ssid}."),
        4..=6 | 15..=17 => format!("Joined {ssid} but couldn't get an IP address from it."),
        53 => format!("{ssid} is no longer in range."),
        r => format!("Couldn't join {ssid} (NetworkManager reason {r})."),
    }
}

/// Public IP, channel and link rate of the current Wi-Fi connection.
pub async fn details(conn: &Connection) -> Details {
    let (public_ip, radio) = tokio::join!(public_ip(), radio_details(conn));
    Details {
        public_ip,
        channel: radio.0,
        tx_rate: radio.1,
    }
}

async fn radio_details(conn: &Connection) -> (Option<String>, Option<String>) {
    let Ok(Some(path)) = device(conn).await else {
        return (None, None);
    };
    let Ok(wireless) = open!(DeviceWirelessProxy, conn, &path) else {
        return (None, None);
    };
    // Without an active access point NM reports a stale bitrate.
    let Some(ap) = active_ap(conn, &path).await else {
        return (None, None);
    };
    let channel = channel_text(conn, &ap).await;
    let tx_rate = wireless
        .bitrate()
        .await
        .ok()
        .filter(|kbps| *kbps > 0)
        .map(|kbps| format!("{} Mbps", kbps / 1000));
    (channel, tx_rate)
}

async fn channel_text(conn: &Connection, ap: &str) -> Option<String> {
    let path = OwnedObjectPath::try_from(ap).ok()?;
    let frequency = read_ap(conn, &path).await.ok()?.frequency;
    let band = match band_of(frequency)? {
        Band::G2 => "2.4 GHz",
        Band::G5 => "5 GHz",
        Band::G6 => "6 GHz",
    };
    Some(format!("{} ({band})", channel_of(frequency)?))
}

async fn public_ip() -> Option<String> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(6))
        .build()
        .ok()?;
    let meta: serde_json::Value = client
        .get("https://speed.cloudflare.com/meta")
        .header("Referer", "https://speed.cloudflare.com/")
        .send()
        .await
        .ok()?
        .json()
        .await
        .ok()?;
    meta.get("clientIp")?.as_str().map(str::to_string)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ap(ssid: Option<&str>, strength: u8, frequency: u32) -> Ap {
        Ap {
            path: format!("/ap/{}/{strength}", ssid.unwrap_or("hidden")),
            ssid: ssid.map(str::to_string),
            strength,
            security: Security::Personal,
            frequency,
        }
    }

    #[test]
    fn security_from_flags() {
        assert_eq!(security_of(0, 0, 0), Security::Open);
        assert_eq!(security_of(1, 0, 0), Security::Wep);
        assert_eq!(security_of(1, 0, KEY_MGMT_PSK), Security::Personal);
        assert_eq!(
            security_of(1, KEY_MGMT_PSK, KEY_MGMT_PSK | KEY_MGMT_SAE),
            Security::Personal
        );
        assert_eq!(security_of(1, 0, KEY_MGMT_SAE), Security::Wpa3Personal);
        assert_eq!(security_of(1, 0, KEY_MGMT_802_1X), Security::Enterprise);
    }

    #[test]
    fn bands_and_channels() {
        assert_eq!(band_of(2437), Some(Band::G2));
        assert_eq!(channel_of(2437), Some(6));
        assert_eq!(band_of(5180), Some(Band::G5));
        assert_eq!(channel_of(5180), Some(36));
        assert_eq!(band_of(5955), Some(Band::G6));
        assert_eq!(channel_of(5955), Some(1));
    }

    #[test]
    fn groups_by_ssid_strongest_first() {
        let aps = [
            ap(Some("Home"), 40, 2437),
            ap(Some("Home"), 80, 5180),
            ap(Some("Cafe"), 60, 2437),
            ap(None, 20, 2437),
            ap(None, 30, 2437),
        ];
        let saved = ["Home".to_string()];
        let active = aps[0].path.clone();
        let networks = group_networks(&aps, Some(&active), &saved);
        let ids: Vec<_> = networks.iter().map(|n| n.id.as_str()).collect();
        assert_eq!(ids, ["Home", "Cafe", "hidden"]);
        assert_eq!(networks[0].strength, 80);
        assert_eq!(networks[0].band, Some(Band::G5));
        assert!(networks[0].saved && networks[0].connected);
        assert!(!networks[1].saved && !networks[1].connected);
        assert_eq!(networks[2].ssid, None);
        assert_eq!(networks[2].strength, 30);
    }

    #[test]
    fn new_profile_key_management() {
        let wpa2 = new_profile("Home", Security::Personal, Some("pw")).unwrap();
        assert_eq!(wpa2[SECURITY_SETTING]["key-mgmt"], Value::from("wpa-psk"));
        assert_eq!(wpa2[SECURITY_SETTING]["psk"], Value::from("pw"));
        let wpa3 = new_profile("Home", Security::Wpa3Personal, Some("pw")).unwrap();
        assert_eq!(wpa3[SECURITY_SETTING]["key-mgmt"], Value::from("sae"));
        let open = new_profile("Cafe", Security::Open, None).unwrap();
        assert!(!open.contains_key(SECURITY_SETTING));
        assert!(new_profile("Home", Security::Personal, None).is_err());
        assert!(new_profile("Corp", Security::Enterprise, Some("pw")).is_err());
    }

    #[test]
    fn failure_sentences() {
        assert_eq!(failure_message("Home", 7), "Wrong password for Home.");
        assert_eq!(failure_message("Home", 9), "Wrong password for Home.");
        assert!(failure_message("Home", 16).contains("IP address"));
        assert!(failure_message("Home", 99).contains("reason 99"));
    }
}
