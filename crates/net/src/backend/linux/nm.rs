//! Hand-written D-Bus proxies for the parts of NetworkManager we use, plus
//! small helpers shared by the other files.

use std::collections::HashMap;
use zbus::proxy;
use zbus::zvariant::{ObjectPath, OwnedObjectPath, OwnedValue, Value};

/// A connection profile: setting name -> key -> value.
pub type Settings = HashMap<String, HashMap<String, OwnedValue>>;
/// Properties of one setting or IP config entry.
pub type Dict = HashMap<String, OwnedValue>;

// NMDeviceType
pub const DEVICE_ETHERNET: u32 = 1;
pub const DEVICE_WIFI: u32 = 2;
pub const DEVICE_MODEM: u32 = 8;
pub const DEVICE_WIREGUARD: u32 = 29;

// NMDeviceState
pub const STATE_ACTIVATED: u32 = 100;
pub const STATE_FAILED: u32 = 120;

/// NM_ACTIVATION_STATE_FLAG_EXTERNAL: the connection was not made by NM.
pub const ACTIVE_FLAG_EXTERNAL: u32 = 0x80;

#[proxy(
    interface = "org.freedesktop.NetworkManager",
    default_service = "org.freedesktop.NetworkManager",
    default_path = "/org/freedesktop/NetworkManager"
)]
pub trait NetworkManager {
    fn get_devices(&self) -> zbus::Result<Vec<OwnedObjectPath>>;

    fn activate_connection(
        &self,
        connection: &ObjectPath<'_>,
        device: &ObjectPath<'_>,
        specific_object: &ObjectPath<'_>,
    ) -> zbus::Result<OwnedObjectPath>;

    fn add_and_activate_connection(
        &self,
        connection: HashMap<&str, HashMap<&str, Value<'_>>>,
        device: &ObjectPath<'_>,
        specific_object: &ObjectPath<'_>,
    ) -> zbus::Result<(OwnedObjectPath, OwnedObjectPath)>;

    fn deactivate_connection(&self, active_connection: &ObjectPath<'_>) -> zbus::Result<()>;

    #[zbus(property)]
    fn wireless_enabled(&self) -> zbus::Result<bool>;
    #[zbus(property)]
    fn set_wireless_enabled(&self, enabled: bool) -> zbus::Result<()>;

    #[zbus(property)]
    fn active_connections(&self) -> zbus::Result<Vec<OwnedObjectPath>>;
}

#[proxy(
    interface = "org.freedesktop.NetworkManager.Device",
    default_service = "org.freedesktop.NetworkManager"
)]
pub trait Device {
    fn reapply(&self, connection: Settings, version_id: u64, flags: u32) -> zbus::Result<()>;

    #[zbus(signal, name = "StateChanged")]
    fn device_state_changed(&self, new_state: u32, old_state: u32, reason: u32)
    -> zbus::Result<()>;

    #[zbus(property)]
    fn interface(&self) -> zbus::Result<String>;
    #[zbus(property)]
    fn driver(&self) -> zbus::Result<String>;
    #[zbus(property)]
    fn device_type(&self) -> zbus::Result<u32>;
    #[zbus(property)]
    fn state(&self) -> zbus::Result<u32>;
    #[zbus(property)]
    fn managed(&self) -> zbus::Result<bool>;
    #[zbus(property, name = "Ip4Config")]
    fn ip4_config(&self) -> zbus::Result<OwnedObjectPath>;
    #[zbus(property)]
    fn active_connection(&self) -> zbus::Result<OwnedObjectPath>;
    #[zbus(property)]
    fn available_connections(&self) -> zbus::Result<Vec<OwnedObjectPath>>;
}

#[proxy(
    interface = "org.freedesktop.NetworkManager.Device.Wired",
    default_service = "org.freedesktop.NetworkManager"
)]
pub trait DeviceWired {
    /// Link speed in Mbit/s, 0 when unknown.
    #[zbus(property)]
    fn speed(&self) -> zbus::Result<u32>;
}

#[proxy(
    interface = "org.freedesktop.NetworkManager.Device.Wireless",
    default_service = "org.freedesktop.NetworkManager"
)]
pub trait DeviceWireless {
    fn get_all_access_points(&self) -> zbus::Result<Vec<OwnedObjectPath>>;
    fn request_scan(&self, options: HashMap<&str, Value<'_>>) -> zbus::Result<()>;

    #[zbus(property)]
    fn active_access_point(&self) -> zbus::Result<OwnedObjectPath>;
    /// Current rate in kbit/s.
    #[zbus(property)]
    fn bitrate(&self) -> zbus::Result<u32>;
    /// CLOCK_BOOTTIME milliseconds of the last finished scan.
    #[zbus(property)]
    fn last_scan(&self) -> zbus::Result<i64>;
}

#[proxy(
    interface = "org.freedesktop.NetworkManager.AccessPoint",
    default_service = "org.freedesktop.NetworkManager"
)]
pub trait AccessPoint {
    #[zbus(property)]
    fn flags(&self) -> zbus::Result<u32>;
    #[zbus(property)]
    fn wpa_flags(&self) -> zbus::Result<u32>;
    #[zbus(property)]
    fn rsn_flags(&self) -> zbus::Result<u32>;
    #[zbus(property)]
    fn ssid(&self) -> zbus::Result<Vec<u8>>;
    #[zbus(property)]
    fn frequency(&self) -> zbus::Result<u32>;
    #[zbus(property)]
    fn strength(&self) -> zbus::Result<u8>;
}

#[proxy(
    interface = "org.freedesktop.NetworkManager.IP4Config",
    default_service = "org.freedesktop.NetworkManager"
)]
pub trait Ip4Config {
    #[zbus(property)]
    fn address_data(&self) -> zbus::Result<Vec<Dict>>;
    #[zbus(property)]
    fn gateway(&self) -> zbus::Result<String>;
    #[zbus(property)]
    fn nameserver_data(&self) -> zbus::Result<Vec<Dict>>;
}

#[proxy(
    interface = "org.freedesktop.NetworkManager.Settings",
    default_service = "org.freedesktop.NetworkManager",
    default_path = "/org/freedesktop/NetworkManager/Settings"
)]
pub trait NmSettings {
    fn list_connections(&self) -> zbus::Result<Vec<OwnedObjectPath>>;
}

#[proxy(
    interface = "org.freedesktop.NetworkManager.Settings.Connection",
    default_service = "org.freedesktop.NetworkManager"
)]
pub trait SettingsConnection {
    fn get_settings(&self) -> zbus::Result<Settings>;
    fn get_secrets(&self, setting_name: &str) -> zbus::Result<Settings>;
    fn update2(
        &self,
        settings: Settings,
        flags: u32,
        args: HashMap<&str, Value<'_>>,
    ) -> zbus::Result<HashMap<String, OwnedValue>>;
    fn delete(&self) -> zbus::Result<()>;

    #[zbus(property)]
    fn unsaved(&self) -> zbus::Result<bool>;
}

#[proxy(
    interface = "org.freedesktop.NetworkManager.Connection.Active",
    default_service = "org.freedesktop.NetworkManager"
)]
pub trait ConnectionActive {
    #[zbus(property, name = "Connection")]
    fn settings_connection(&self) -> zbus::Result<OwnedObjectPath>;
    #[zbus(property)]
    fn uuid(&self) -> zbus::Result<String>;
    #[zbus(property, name = "Type")]
    fn connection_type(&self) -> zbus::Result<String>;
    #[zbus(property)]
    fn state_flags(&self) -> zbus::Result<u32>;
    #[zbus(property, name = "Default")]
    fn is_default(&self) -> zbus::Result<bool>;
    #[zbus(property)]
    fn devices(&self) -> zbus::Result<Vec<OwnedObjectPath>>;
}

/// Open a proxy for `path` without property caching: we rebuild snapshots
/// from scratch, so a cache would only add match rules and tasks.
macro_rules! open {
    ($proxy:ident, $conn:expr, $path:expr) => {{
        let opened: zbus::Result<$proxy> = async {
            $proxy::builder($conn)
                .path($path.clone())?
                .cache_properties(zbus::proxy::CacheProperties::No)
                .build()
                .await
        }
        .await;
        opened
    }};
}
pub(crate) use open;

/// The NetworkManager root object, without property caching.
pub async fn manager(conn: &zbus::Connection) -> zbus::Result<NetworkManagerProxy<'static>> {
    NetworkManagerProxy::builder(conn)
        .cache_properties(zbus::proxy::CacheProperties::No)
        .build()
        .await
}

/// The Settings object, without property caching.
pub async fn settings(conn: &zbus::Connection) -> zbus::Result<NmSettingsProxy<'static>> {
    NmSettingsProxy::builder(conn)
        .cache_properties(zbus::proxy::CacheProperties::No)
        .build()
        .await
}

/// True for the "/" path NM uses for "nothing".
pub fn is_none(path: &OwnedObjectPath) -> bool {
    path.as_str() == "/"
}

pub fn root() -> OwnedObjectPath {
    OwnedObjectPath::from(ObjectPath::from_static_str_unchecked("/"))
}

/// Turn a D-Bus failure into a sentence for a toast.
pub fn explain(what: &str, error: &zbus::Error) -> String {
    match error {
        zbus::Error::MethodError(name, detail, _) => {
            if name.as_str().ends_with("PermissionDenied")
                || name.as_str().ends_with("AccessDenied")
            {
                return format!(
                    "NetworkManager refused to {what}. Make sure you are in the networkmanager group."
                );
            }
            match detail {
                Some(d) => format!("Couldn't {what}: {d}"),
                None => format!("Couldn't {what}: {name}"),
            }
        }
        zbus::Error::InputOutput(_) => {
            format!("Couldn't {what}: can't reach NetworkManager. Is it running?")
        }
        other => format!("Couldn't {what}: {other}"),
    }
}

pub fn string_of(dict: &Dict, key: &str) -> Option<String> {
    let value = dict.get(key)?;
    value.downcast_ref::<&str>().ok().map(str::to_string)
}

pub fn u32_of(dict: &Dict, key: &str) -> Option<u32> {
    dict.get(key)?.downcast_ref::<u32>().ok()
}
