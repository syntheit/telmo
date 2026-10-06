//! Wi-Fi through CoreWLAN. Reading state and event delivery happen on the
//! backend's run-loop thread; scans, joins and power changes run on workers
//! with their own `CWInterface`.

use super::shell;
use super::wake::Wake;
use crate::model::{Band, Details, Network, Security, Wifi};
use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2::{AnyThread, DefinedClass, define_class, msg_send};
use objc2_core_wlan::{
    CWChannel, CWChannelBand, CWChannelWidth, CWEventDelegate, CWEventType, CWInterface, CWNetwork,
    CWSecurity, CWWiFiClient,
};
use objc2_foundation::{
    NSData, NSDictionary, NSError, NSKeyedArchiveRootObjectKey, NSKeyedUnarchiver, NSObject,
    NSObjectProtocol, NSString,
};
use std::time::Duration;

const SCAN_FAILED: &str = "Couldn't scan for networks";
/// CoreWLAN error codes that mean the credentials were rejected.
const WRONG_PASSWORD_CODES: [isize; 4] = [-3924, -3925, -3912, -3900];

struct Events {
    changed: Wake,
}

define_class!(
    #[unsafe(super(NSObject))]
    #[ivars = Events]
    struct Delegate;

    unsafe impl NSObjectProtocol for Delegate {}

    unsafe impl CWEventDelegate for Delegate {
        #[unsafe(method(powerStateDidChangeForWiFiInterfaceWithName:))]
        fn power_changed(&self, _name: &NSString) {
            self.notify();
        }

        #[unsafe(method(ssidDidChangeForWiFiInterfaceWithName:))]
        fn ssid_changed(&self, _name: &NSString) {
            self.notify();
        }

        #[unsafe(method(bssidDidChangeForWiFiInterfaceWithName:))]
        fn bssid_changed(&self, _name: &NSString) {
            self.notify();
        }

        #[unsafe(method(linkDidChangeForWiFiInterfaceWithName:))]
        fn link_changed(&self, _name: &NSString) {
            self.notify();
        }

        #[unsafe(method(scanCacheUpdatedForWiFiInterfaceWithName:))]
        fn scan_cache_updated(&self, _name: &NSString) {
            self.notify();
        }
    }
);

impl Delegate {
    fn new(changed: Wake) -> Retained<Self> {
        let this = Self::alloc().set_ivars(Events { changed });
        unsafe { msg_send![super(this), init] }
    }

    fn notify(&self) {
        self.ivars().changed.send();
    }
}

/// The Wi-Fi radio as seen from the run-loop thread.
pub struct Radio {
    pub device: String,
    iface: Retained<CWInterface>,
    // The client keeps its delegate weakly, so both live as long as the radio.
    _client: Retained<CWWiFiClient>,
    _delegate: Retained<Delegate>,
}

impl Radio {
    /// None when this Mac has no Wi-Fi hardware.
    pub fn start(changed: Wake) -> Option<Radio> {
        let client = unsafe { CWWiFiClient::sharedWiFiClient() };
        let iface = unsafe { client.interface() }?;
        let device = unsafe { iface.interfaceName() }?.to_string();
        let delegate = Delegate::new(changed);
        let object: &AnyObject = &delegate;
        unsafe { client.setDelegate(Some(object)) };
        for event in [
            CWEventType::PowerDidChange,
            CWEventType::SSIDDidChange,
            CWEventType::BSSIDDidChange,
            CWEventType::LinkDidChange,
            CWEventType::ScanCacheUpdated,
        ] {
            // Without events we still refresh on dynamic-store changes.
            let _ = unsafe { client.startMonitoringEventWithType_error(event) };
        }
        Some(Radio {
            device,
            iface,
            _client: client,
            _delegate: delegate,
        })
    }

    pub fn power(&self) -> bool {
        unsafe { self.iface.powerOn() }
    }

    /// Wi-Fi state from CoreWLAN's cached scan results.
    pub fn read(&self, current: Option<Current>, scanning: bool) -> Wifi {
        let power = self.power();
        let entries = if power {
            cached_entries(&self.iface)
        } else {
            vec![]
        };
        let saved = saved_ssids(&self.iface);
        let current = current.filter(|_| power);
        let view = assemble(entries, current, &saved);
        Wifi {
            interface: self.device.clone(),
            power,
            scanning,
            networks: view.networks,
            saved_elsewhere: view.saved_elsewhere,
            names_hidden: view.names_hidden,
        }
    }

    /// The connected network, with its name even when Location is off.
    pub fn current(&self, record: Option<ScanRecord>) -> Option<Current> {
        if !self.power() {
            return None;
        }
        let ssid = unsafe { self.iface.ssid() }
            .map(|s| s.to_string())
            .filter(|s| !s.is_empty())
            .or_else(|| record.as_ref().and_then(|r| r.ssid.clone()));
        let channel = unsafe { self.iface.wlanChannel() };
        // rssi is 0 and there's no channel when not associated.
        let rssi = unsafe { self.iface.rssiValue() };
        if ssid.is_none() && (rssi == 0 || channel.is_none()) {
            return None;
        }
        Some(Current {
            ssid,
            rssi,
            channel: channel.as_ref().map(|c| unsafe { c.channelNumber() }),
            band: channel
                .as_ref()
                .and_then(|c| band(unsafe { c.channelBand() })),
            security: security_of_interface(&self.iface),
        })
    }
}

/// What the connected network looks like.
#[derive(Debug, Clone)]
pub struct Current {
    /// None when macOS hides the name (no Location permission).
    pub ssid: Option<String>,
    pub rssi: isize,
    pub channel: Option<isize>,
    pub band: Option<Band>,
    pub security: Security,
}

/// One access point from a scan, before deduplication.
#[derive(Debug, Clone)]
pub struct Entry {
    pub connected: bool,
    pub ssid: Option<String>,
    pub bssid: Option<String>,
    pub rssi: isize,
    pub channel: isize,
    pub band: Option<Band>,
    pub security: Security,
}

impl Entry {
    fn id(&self) -> String {
        match (&self.ssid, &self.bssid) {
            (Some(ssid), _) => ssid.clone(),
            (None, Some(bssid)) => bssid.clone(),
            (None, None) => format!("hidden-{}-{}", self.channel, self.rssi),
        }
    }
}

fn entry(network: &CWNetwork) -> Entry {
    let channel = unsafe { network.wlanChannel() };
    Entry {
        connected: false,
        ssid: unsafe { network.ssid() }
            .map(|s| s.to_string())
            .filter(|s| !s.is_empty()),
        bssid: unsafe { network.bssid() }.map(|s| s.to_string()),
        rssi: unsafe { network.rssiValue() },
        channel: channel.as_ref().map_or(0, |c| unsafe { c.channelNumber() }),
        band: channel
            .as_ref()
            .and_then(|c| band(unsafe { c.channelBand() })),
        security: security_of_network(network),
    }
}

fn cached_entries(iface: &CWInterface) -> Vec<Entry> {
    let results = unsafe { iface.cachedScanResults() };
    let networks = results.map(|set| set.to_vec()).unwrap_or_default();
    networks.iter().map(|n| entry(n)).collect()
}

fn saved_ssids(iface: &CWInterface) -> Vec<String> {
    let Some(config) = (unsafe { iface.configuration() }) else {
        return vec![];
    };
    let profiles = unsafe { config.networkProfiles() }.array();
    profiles
        .iter()
        .filter_map(|p| unsafe { p.ssid() })
        .map(|s| s.to_string())
        .collect()
}

fn band(band: CWChannelBand) -> Option<Band> {
    match band {
        CWChannelBand::Band2GHz => Some(Band::G2),
        CWChannelBand::Band5GHz => Some(Band::G5),
        CWChannelBand::Band6GHz => Some(Band::G6),
        _ => None,
    }
}

fn security_of_network(network: &CWNetwork) -> Security {
    let supports = |s| unsafe { network.supportsSecurity(s) };
    security_from(supports)
}

fn security_of_interface(iface: &CWInterface) -> Security {
    let current = unsafe { iface.security() };
    security_from(|s| s == current)
}

/// Strongest-first check of what a network supports.
fn security_from(supports: impl Fn(CWSecurity) -> bool) -> Security {
    let enterprise = [
        CWSecurity::WPA3Enterprise,
        CWSecurity::WPA2Enterprise,
        CWSecurity::WPAEnterprise,
        CWSecurity::WPAEnterpriseMixed,
        CWSecurity::Enterprise,
        CWSecurity::DynamicWEP,
    ];
    if enterprise.into_iter().any(&supports) {
        return Security::Enterprise;
    }
    if supports(CWSecurity::WPA3Personal) || supports(CWSecurity::WPA3Transition) {
        return Security::Wpa3Personal;
    }
    let personal = [
        CWSecurity::WPA2Personal,
        CWSecurity::WPAPersonal,
        CWSecurity::WPAPersonalMixed,
        CWSecurity::Personal,
    ];
    if personal.into_iter().any(&supports) {
        return Security::Personal;
    }
    if supports(CWSecurity::WEP) {
        return Security::Wep;
    }
    if supports(CWSecurity::None)
        || supports(CWSecurity::OWE)
        || supports(CWSecurity::OWETransition)
    {
        return Security::Open;
    }
    Security::Personal
}

pub struct View {
    pub networks: Vec<Network>,
    pub saved_elsewhere: Vec<String>,
    pub names_hidden: bool,
}

/// Merge scan entries with the connected network and saved profiles.
pub fn assemble(mut entries: Vec<Entry>, current: Option<Current>, saved: &[String]) -> View {
    let names_hidden = !entries.is_empty() && entries.iter().all(|e| e.ssid.is_none());
    entries.sort_by_key(|e| std::cmp::Reverse(e.rssi));
    let mut seen = std::collections::HashSet::new();
    entries.retain(|e| seen.insert(e.id()));
    if let Some(current) = &current {
        mark_connected(&mut entries, current);
    }
    let networks: Vec<Network> = entries
        .iter()
        .map(|e| Network {
            id: e.id(),
            ssid: e.ssid.clone(),
            strength: strength(e.rssi),
            security: e.security,
            band: e.band,
            saved: e.ssid.as_ref().is_some_and(|s| saved.contains(s)),
            connected: e.connected,
        })
        .collect();
    let mut saved_elsewhere: Vec<String> = Vec::new();
    for ssid in saved {
        let in_range = networks.iter().any(|n| n.ssid.as_ref() == Some(ssid));
        if !in_range && !saved_elsewhere.contains(ssid) {
            saved_elsewhere.push(ssid.clone());
        }
    }
    View {
        networks,
        saved_elsewhere,
        names_hidden,
    }
}

/// Mark the connected network in the list, adding it if the scan missed it.
/// When names are hidden, the best match is the unnamed entry on the same
/// channel with the closest signal.
fn mark_connected(entries: &mut Vec<Entry>, current: &Current) {
    let named = current
        .ssid
        .as_ref()
        .and_then(|ssid| entries.iter().position(|e| e.ssid.as_ref() == Some(ssid)));
    match named.or_else(|| closest_unnamed(entries, current)) {
        Some(i) => {
            entries[i].connected = true;
            if entries[i].ssid.is_none() {
                entries[i].ssid = current.ssid.clone();
            }
        }
        None => entries.push(Entry {
            connected: true,
            ssid: current.ssid.clone(),
            bssid: None,
            rssi: current.rssi,
            channel: current.channel.unwrap_or(0),
            band: current.band,
            security: current.security,
        }),
    }
}

fn closest_unnamed(entries: &[Entry], current: &Current) -> Option<usize> {
    entries
        .iter()
        .enumerate()
        .filter(|(_, e)| e.ssid.is_none() && Some(e.channel) == current.channel)
        .min_by_key(|(_, e)| e.rssi.abs_diff(current.rssi))
        .map(|(i, _)| i)
}

/// -90 dBm and below is 0, -40 dBm and above is 100.
pub fn strength(rssi: isize) -> u8 {
    ((rssi + 90) * 2).clamp(0, 100) as u8
}

/// The record macOS keeps for the connected network, readable without Location.
#[derive(Debug, Clone)]
pub struct ScanRecord {
    pub ssid: Option<String>,
}

/// Decode the archived `CachedScanRecord` from the dynamic store.
pub fn scan_record(archived: &[u8]) -> Option<ScanRecord> {
    let decode = || unsafe {
        let data = NSData::with_bytes(archived);
        let unarchiver =
            NSKeyedUnarchiver::initForReadingFromData_error(NSKeyedUnarchiver::alloc(), &data)
                .ok()?;
        unarchiver.setRequiresSecureCoding(false);
        let root = unarchiver.decodeObjectForKey(NSKeyedArchiveRootObjectKey)?;
        let dict = root.downcast_ref::<NSDictionary>()?;
        Some(record_from(dict))
    };
    // Unarchiving raises an Objective-C exception on malformed data.
    objc2::exception::catch(std::panic::AssertUnwindSafe(decode)).ok()?
}

fn record_from(dict: &NSDictionary) -> ScanRecord {
    let get = |key: &str| dict.objectForKey(&*NSString::from_str(key));
    let text = get("SSID_STR")
        .and_then(|v| v.downcast_ref::<NSString>().map(|s| s.to_string()))
        .filter(|s| !s.is_empty());
    let bytes = || {
        let value = get("SSID")?;
        let data = value.downcast_ref::<NSData>()?;
        String::from_utf8(data.to_vec())
            .ok()
            .filter(|s| !s.is_empty())
    };
    ScanRecord {
        ssid: text.or_else(bytes),
    }
}

// Worker-thread operations. Each makes its own CWInterface.

/// BSD name of the Mac's Wi-Fi interface, if it has one.
pub fn default_device() -> Option<String> {
    let client = unsafe { CWWiFiClient::sharedWiFiClient() };
    let iface = unsafe { client.interface() }?;
    unsafe { iface.interfaceName() }.map(|n| n.to_string())
}

fn interface(device: &str) -> Option<Retained<CWInterface>> {
    let client = unsafe { CWWiFiClient::sharedWiFiClient() };
    unsafe { client.interfaceWithName(Some(&NSString::from_str(device))) }
}

fn missing(device: &str) -> String {
    format!("Couldn't open the Wi-Fi interface {device}.")
}

/// Run a fresh scan. CoreWLAN stores the results in its cache.
pub fn scan(device: &str) -> Result<(), String> {
    let iface = interface(device).ok_or_else(|| missing(device))?;
    if !unsafe { iface.powerOn() } {
        return Err("Wi-Fi is off. Turn it on to scan.".to_string());
    }
    unsafe { iface.scanForNetworksWithName_error(None) }
        .map(|_| ())
        .map_err(|e| format!("{SCAN_FAILED}: {}.", describe(&e)))
}

pub fn set_power(device: &str, on: bool) -> Result<(), String> {
    let iface = interface(device).ok_or_else(|| missing(device))?;
    unsafe { iface.setPower_error(on) }.map_err(|e| {
        let state = if on { "on" } else { "off" };
        format!("Couldn't turn Wi-Fi {state}: {}.", describe(&e))
    })
}

/// Join the network with this id (see `Entry::id`).
pub fn join(device: &str, id: &str, password: Option<&str>) -> Result<String, String> {
    let iface = interface(device).ok_or_else(|| missing(device))?;
    let network = find_network(&iface, id)
        .ok_or_else(|| format!("{id} isn't in range any more. Rescan and try again."))?;
    let name = unsafe { network.ssid() }.map_or_else(|| id.to_string(), |s| s.to_string());
    let secret = password.map(NSString::from_str);
    match unsafe { iface.associateToNetwork_password_error(&network, secret.as_deref()) } {
        Ok(()) => Ok(format!("Joined {name}.")),
        Err(e) => Err(join_error(
            &e,
            &name,
            password.is_some(),
            entry(&network).security,
        )),
    }
}

fn find_network(iface: &CWInterface, id: &str) -> Option<Retained<CWNetwork>> {
    let cached = unsafe { iface.cachedScanResults() }.map(|set| set.to_vec());
    let found = cached.into_iter().flatten().find(|n| entry(n).id() == id);
    if found.is_some() {
        return found;
    }
    let scanned = unsafe { iface.scanForNetworksWithName_error(Some(&NSString::from_str(id))) };
    scanned.ok()?.to_vec().into_iter().next()
}

fn join_error(error: &NSError, ssid: &str, had_password: bool, security: Security) -> String {
    let code = error.code();
    if had_password && WRONG_PASSWORD_CODES.contains(&code) {
        return format!("Wrong password for {ssid}.");
    }
    if !had_password && security.needs_password() {
        return format!("Enter the password for {ssid}.");
    }
    format!("Couldn't join {ssid}: {}.", describe(error))
}

fn describe(error: &NSError) -> String {
    let text = error.localizedDescription().to_string();
    text.trim_end_matches('.').to_string()
}

/// Remove a saved network (asks for an administrator password).
pub fn forget(device: &str, ssid: &str) -> Result<(), String> {
    let command = vec![
        "/usr/sbin/networksetup".to_string(),
        "-removepreferredwirelessnetwork".to_string(),
        device.to_string(),
        ssid.to_string(),
    ];
    shell::admin(&[command])
}

/// The saved password from the keychain. macOS may ask to unlock it.
pub fn reveal_password(ssid: &str) -> Result<String, String> {
    let args = ["find-generic-password", "-wa", ssid];
    let out = shell::run("/usr/bin/security", &args, Duration::from_secs(60))?;
    if out.ok {
        return Ok(out.stdout.trim_end_matches('\n').to_string());
    }
    if out.stderr.contains("could not be found") {
        return Err(format!("No saved password for {ssid} in the keychain."));
    }
    if out.stderr.contains("User canceled") || out.stderr.contains("denied") {
        return Err("Cancelled.".to_string());
    }
    Err(format!("Couldn't read the keychain: {}", out.stderr.trim()))
}

/// Channel and transmit rate of the current connection.
pub fn details(device: &str) -> Details {
    let Some(iface) = interface(device) else {
        return Details::default();
    };
    let channel = unsafe { iface.wlanChannel() }.map(|c| channel_text(&c));
    let rate = unsafe { iface.transmitRate() };
    Details {
        public_ip: None,
        channel,
        tx_rate: (rate > 0.0).then(|| format!("{rate:.0} Mbps")),
    }
}

fn channel_text(channel: &CWChannel) -> String {
    let number = unsafe { channel.channelNumber() };
    let band = match unsafe { channel.channelBand() } {
        CWChannelBand::Band2GHz => "2.4 GHz",
        CWChannelBand::Band5GHz => "5 GHz",
        CWChannelBand::Band6GHz => "6 GHz",
        _ => "",
    };
    let width = match unsafe { channel.channelWidth() } {
        CWChannelWidth::Width20MHz => "20 MHz",
        CWChannelWidth::Width40MHz => "40 MHz",
        CWChannelWidth::Width80MHz => "80 MHz",
        CWChannelWidth::Width160MHz => "160 MHz",
        _ => "",
    };
    let parts = [number.to_string(), band.to_string(), width.to_string()];
    let parts: Vec<_> = parts.into_iter().filter(|p| !p.is_empty()).collect();
    parts.join(" · ")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(ssid: Option<&str>, rssi: isize, channel: isize) -> Entry {
        Entry {
            connected: false,
            ssid: ssid.map(str::to_string),
            bssid: None,
            rssi,
            channel,
            band: Some(Band::G5),
            security: Security::Personal,
        }
    }

    fn current(ssid: &str, rssi: isize, channel: isize) -> Current {
        Current {
            ssid: Some(ssid.to_string()),
            rssi,
            channel: Some(channel),
            band: Some(Band::G5),
            security: Security::Personal,
        }
    }

    #[test]
    fn maps_rssi_to_strength() {
        assert_eq!(strength(-95), 0);
        assert_eq!(strength(-65), 50);
        assert_eq!(strength(-30), 100);
    }

    #[test]
    fn dedupes_by_ssid_keeping_strongest() {
        let entries = vec![entry(Some("a"), -70, 1), entry(Some("a"), -50, 36)];
        let view = assemble(entries, None, &[]);
        assert_eq!(view.networks.len(), 1);
        assert_eq!(view.networks[0].strength, 80);
        assert!(!view.names_hidden);
    }

    #[test]
    fn hidden_names_get_the_connected_name() {
        let entries = vec![entry(None, -45, 149), entry(None, -80, 36)];
        let view = assemble(
            entries,
            Some(current("Home", -47, 149)),
            &["Home".into(), "Away".into()],
        );
        assert!(view.names_hidden);
        assert_eq!(view.networks[0].ssid.as_deref(), Some("Home"));
        assert!(view.networks[0].connected && view.networks[0].saved);
        assert!(!view.networks[1].connected);
        assert_eq!(view.saved_elsewhere, vec!["Away".to_string()]);
    }

    #[test]
    fn connected_network_is_listed_even_when_not_scanned() {
        let view = assemble(vec![], Some(current("Home", -60, 6)), &[]);
        assert_eq!(view.networks.len(), 1);
        assert!(view.networks[0].connected);
    }
}
