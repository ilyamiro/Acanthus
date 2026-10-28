use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use acanthus_core::{Signal, batch};
use acanthus_io::IoError;
use calloop::LoopHandle;
use futures_lite::StreamExt;
use zbus::Connection;
use zbus::{OwnedVariant as OwnedValue, Variant as ZValue};

use crate::platform::bridge::{EventSink, ServiceHandle, spawn_supervised};
use crate::platform::dbus::BusKind;
use crate::platform::network_import::{ImportError, ImportOptions, import_profiles};

const SERVICE: &str = "org.freedesktop.NetworkManager";
const ROOT_PATH: &str = "/org/freedesktop/NetworkManager";
const SETTINGS_PATH: &str = "/org/freedesktop/NetworkManager/Settings";
const MANAGER_IFACE: &str = "org.freedesktop.NetworkManager";
const DEVICE_IFACE: &str = "org.freedesktop.NetworkManager.Device";
const WIRED_IFACE: &str = "org.freedesktop.NetworkManager.Device.Wired";
const WIRELESS_IFACE: &str = "org.freedesktop.NetworkManager.Device.Wireless";
const AP_IFACE: &str = "org.freedesktop.NetworkManager.AccessPoint";
const SETTINGS_IFACE: &str = "org.freedesktop.NetworkManager.Settings";
const ACTIVE_IFACE: &str = "org.freedesktop.NetworkManager.Connection.Active";
const CONNECTION_IFACE: &str = "org.freedesktop.NetworkManager.Settings.Connection";

const DEVICE_TYPE_ETHERNET: u32 = 1;
const DEVICE_TYPE_WIFI: u32 = 2;
const DEVICE_STATE_ACTIVATED: u32 = 100;

const AP_FLAGS_PRIVACY: u32 = 0x1;
const KEY_MGMT_PSK: u32 = 0x100;
const KEY_MGMT_8021X: u32 = 0x200;
const KEY_MGMT_SAE: u32 = 0x400;
const KEY_MGMT_OWE: u32 = 0x800;
const KEY_MGMT_EAP_SUITE_B_192: u32 = 0x2000;

const AGENT_PATH: &str = "/org/freedesktop/NetworkManager/SecretAgent";
const AGENT_MANAGER_PATH: &str = "/org/freedesktop/NetworkManager/AgentManager";
const AGENT_MANAGER_IFACE: &str = "org.freedesktop.NetworkManager.AgentManager";
fn agent_identifier(app: &str) -> String {
    format!("org.{app}.secrets")
}
const SECRET_FLAG_ALLOW_INTERACTION: u32 = 0x1;
const SECRET_FLAG_REQUEST_NEW: u32 = 0x2;
const SCAN_INTERVAL: Duration = Duration::from_secs(10);
const COALESCE: Duration = Duration::from_millis(80);

/// Properties of one D-Bus interface.
pub type Properties = HashMap<String, OwnedValue>;
/// Connection settings by setting name.
pub type ConnectionSettings = HashMap<String, Properties>;

/// Kind of a network device.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DeviceKind {
    /// An Ethernet device.
    Wired,
    /// A Wi-Fi device.
    Wifi,
    /// Any other device.
    #[default]
    Other,
}

/// Security of a Wi-Fi network.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum WifiSecurity {
    /// No encryption.
    #[default]
    Open,
    /// Opportunistic wireless encryption without a password.
    Owe,
    /// WEP.
    Wep,
    /// WPA with a pre-shared key.
    WpaPsk,
    /// WPA2 with a pre-shared key.
    Wpa2Psk,
    /// WPA3 with simultaneous authentication of equals.
    Wpa3Sae,
    /// Enterprise authentication (802.1X).
    Eap,
}

impl WifiSecurity {
    /// True when no password is needed.
    pub fn is_open(self) -> bool {
        matches!(self, WifiSecurity::Open | WifiSecurity::Owe)
    }

    /// True when a password or key is needed.
    pub fn needs_psk(self) -> bool {
        matches!(self, WifiSecurity::Wep | WifiSecurity::WpaPsk | WifiSecurity::Wpa2Psk | WifiSecurity::Wpa3Sae)
    }
}

/// Outer authentication method of an enterprise network.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum EapMethod {
    /// PEAP.
    #[default]
    Peap,
    /// TTLS.
    Ttls,
    /// TLS.
    Tls,
    /// FAST.
    Fast,
    /// PWD.
    Pwd,
    /// SIM.
    Sim,
    /// AKA.
    Aka,
    /// AKA'.
    AkaPrime,
}

/// How EAP-FAST provisions its credentials.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum FastProvisioning {
    /// Anonymous provisioning.
    Anonymous,
    /// Authenticated provisioning.
    Authenticated,
    /// Both (the default).
    #[default]
    Both,
}

impl FastProvisioning {
    fn code(self) -> &'static str {
        match self {
            FastProvisioning::Anonymous => "1",
            FastProvisioning::Authenticated => "2",
            FastProvisioning::Both => "3",
        }
    }
}

impl EapMethod {
    fn name(self) -> &'static str {
        match self {
            EapMethod::Peap => "peap",
            EapMethod::Ttls => "ttls",
            EapMethod::Tls => "tls",
            EapMethod::Fast => "fast",
            EapMethod::Pwd => "pwd",
            EapMethod::Sim => "sim",
            EapMethod::Aka => "aka",
            EapMethod::AkaPrime => "aka'",
        }
    }
}

/// Credentials of an enterprise network.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Enterprise {
    /// Outer method.
    pub method: EapMethod,
    /// User identity.
    pub identity: String,
    /// Anonymous outer identity.
    pub anonymous_identity: Option<String>,
    /// Password.
    pub password: Option<String>,
    /// Inner authentication method.
    pub phase2: Option<String>,
    /// Path of the CA certificate.
    pub ca_cert: Option<String>,
    /// Path of the client certificate.
    pub client_cert: Option<String>,
    /// Path of the private key.
    pub private_key: Option<String>,
    /// Password of the private key.
    pub private_key_password: Option<String>,
    /// Required suffix of the server's domain.
    pub domain_suffix: Option<String>,
    /// Required match of the server's domain.
    pub domain_match: Option<String>,
    /// Required match of the certificate subject.
    pub subject_match: Option<String>,
    /// Required matches of alternative subject names.
    pub altsubject_matches: Vec<String>,
    /// True to trust the system CA certificates.
    pub system_ca_certs: bool,
    /// Directory of CA certificates.
    pub ca_path: Option<String>,
    /// FAST provisioning mode.
    pub fast_provisioning: FastProvisioning,
    /// Path of the FAST PAC file.
    pub pac_file: Option<String>,
    /// Smart card or token PIN.
    pub pin: Option<String>,
}

/// How a WEP key is given.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum WepKeyType {
    /// The key itself.
    #[default]
    Key,
    /// A passphrase the key is derived from.
    Passphrase,
}

/// Credentials and flags for connecting to a network.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct ConnectOptions {
    /// Password or key.
    pub psk: Option<String>,
    /// True for a hidden network.
    pub hidden: bool,
    /// Enterprise credentials.
    pub enterprise: Option<Enterprise>,
    /// How the WEP key is given.
    pub wep_key_type: WepKeyType,
    /// True for shared-key authentication.
    pub wep_shared: bool,
}

/// A network device.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct NetworkDevice {
    /// D-Bus object path.
    pub path: String,
    /// Interface name.
    pub name: String,
    /// Kind of device.
    pub kind: DeviceKind,
    /// Hardware address.
    pub address: String,
    /// True when connected.
    pub connected: bool,
    /// True when a cable is plugged in or the radio is on.
    pub has_link: bool,
}

/// A Wi-Fi network in range.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct WifiNetwork {
    /// D-Bus object path of the access point.
    pub path: String,
    /// Object path of the Wi-Fi device it is seen by.
    pub device: String,
    /// Network name.
    pub name: String,
    /// Signal strength from 0 to 100.
    pub signal_strength: f64,
    /// Security.
    pub security: WifiSecurity,
    /// True when connected to it.
    pub connected: bool,
    /// True when a saved profile exists.
    pub known: bool,
}

/// A request from NetworkManager for a password or key.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct SecretRequest {
    /// Identifier to answer with.
    pub id: u64,
    /// Object path of the connection.
    pub connection_path: String,
    /// Name of the connection.
    pub connection_name: String,
    /// Name of the setting that needs the secret.
    pub setting: String,
    /// Hints from NetworkManager.
    pub hints: Vec<String>,
    /// Keys of the secrets wanted.
    pub keys: Vec<String>,
    /// The first key wanted.
    pub key: String,
    /// Message to show to the user.
    pub message: String,
    /// True when the user may be asked.
    pub allow_interaction: bool,
    /// True when the earlier secret was wrong.
    pub request_new: bool,
}

/// A VPN profile.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct VpnConnection {
    /// D-Bus object path.
    pub path: String,
    /// Name.
    pub name: String,
    /// True when connected.
    pub active: bool,
}

/// Everything known about the network at one moment.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct NetworkSnapshot {
    /// True when NetworkManager is reachable.
    pub available: bool,
    /// True when Wi-Fi is switched on.
    pub wifi_enabled: bool,
    /// Network devices.
    pub devices: Vec<NetworkDevice>,
    /// Wi-Fi networks in range.
    pub networks: Vec<WifiNetwork>,
    /// VPN profiles.
    pub vpns: Vec<VpnConnection>,
}

/// A request to the network service.
#[derive(Debug, Clone, PartialEq)]
pub enum NetworkCommand {
    /// Switches Wi-Fi on or off.
    SetWifiEnabled(bool),
    /// Scans for networks once.
    Scan,
    /// Scans continuously or stops.
    SetScanning(bool),
    /// Answer to a secret request.
    ProvideSecret {
        /// Id of the request.
        id: u64,
        /// The secrets by key, `None` to cancel.
        secrets: Option<HashMap<String, String>>,
    },
    /// Connect to a network, creating a profile when none exists.
    Connect {
        /// Network name.
        ssid: String,
        /// Credentials and flags.
        options: Box<ConnectOptions>,
    },
    /// Disconnects the device at this object path.
    Disconnect(String),
    /// Activates the best saved connection of the device at this object path.
    Activate(String),
    /// Deletes the saved profiles of the network with this name.
    Forget(String),
    /// Adds a connection profile.
    AddConnection(Box<ConnectionSettings>),
    /// Connects the VPN at this object path.
    ConnectVpn(String),
    /// Disconnects the VPN at this object path.
    DisconnectVpn(String),
}

/// An access point as NetworkManager reports it.
#[derive(Debug, Clone, Default)]
pub struct RawAccessPoint {
    /// D-Bus object path.
    pub path: String,
    /// Its properties.
    pub props: Properties,
}

/// A device as NetworkManager reports it.
#[derive(Debug, Clone, Default)]
pub struct RawDevice {
    /// D-Bus object path.
    pub path: String,
    /// Properties of the device interface.
    pub device: Properties,
    /// Properties of the wired interface.
    pub wired: Properties,
    /// Properties of the wireless interface.
    pub wireless: Properties,
    /// Access points seen by the device.
    pub access_points: Vec<RawAccessPoint>,
}

/// Saved profiles by network name: the object paths of the connections.
pub type Known = HashMap<String, Vec<String>>;

/// A VPN profile with its active connection.
#[derive(Debug, Clone, Default)]
pub struct VpnProfile {
    /// D-Bus object path of the profile.
    pub path: String,
    /// Name.
    pub name: String,
    /// Object path of the active connection, if connected.
    pub active_path: Option<String>,
}

/// Saved connection profiles.
#[derive(Debug, Clone, Default)]
pub struct Profiles {
    /// Wi-Fi profiles.
    pub known: Known,
    /// VPN profiles.
    pub vpns: Vec<VpnProfile>,
}

fn as_str(value: &OwnedValue) -> Option<String> {
    match &**value {
        ZValue::Str(s) => Some(s.to_string()),
        ZValue::ObjectPath(p) => Some(p.to_string()),
        _ => None,
    }
}

fn as_bool(value: &OwnedValue) -> Option<bool> {
    match &**value {
        ZValue::Bool(b) => Some(*b),
        _ => None,
    }
}

fn as_u32(value: &OwnedValue) -> Option<u32> {
    match &**value {
        ZValue::U32(n) => Some(*n),
        ZValue::U8(n) => Some(u32::from(*n)),
        ZValue::U16(n) => Some(u32::from(*n)),
        _ => None,
    }
}

fn as_bytes(value: &OwnedValue) -> Option<Vec<u8>> {
    Vec::<u8>::try_from(value.clone()).ok()
}

/// The security of an access point from its flags and its WPA and RSN flags.
pub fn security_from_flags(flags: u32, wpa: u32, rsn: u32) -> WifiSecurity {
    let any = wpa | rsn;
    if any & KEY_MGMT_EAP_SUITE_B_192 != 0 || any & KEY_MGMT_8021X != 0 {
        WifiSecurity::Eap
    } else if any & KEY_MGMT_SAE != 0 {
        WifiSecurity::Wpa3Sae
    } else if rsn & KEY_MGMT_PSK != 0 {
        WifiSecurity::Wpa2Psk
    } else if wpa & KEY_MGMT_PSK != 0 {
        WifiSecurity::WpaPsk
    } else if any & KEY_MGMT_OWE != 0 {
        WifiSecurity::Owe
    } else if flags & AP_FLAGS_PRIVACY != 0 {
        WifiSecurity::Wep
    } else {
        WifiSecurity::Open
    }
}

fn device_kind(code: u32) -> DeviceKind {
    match code {
        DEVICE_TYPE_ETHERNET => DeviceKind::Wired,
        DEVICE_TYPE_WIFI => DeviceKind::Wifi,
        _ => DeviceKind::Other,
    }
}

fn parse_device(raw: &RawDevice) -> Option<NetworkDevice> {
    let name = raw.device.get("Interface").and_then(as_str).unwrap_or_default();
    if name == "lo" {
        return None;
    }
    let kind = device_kind(raw.device.get("DeviceType").and_then(as_u32).unwrap_or(0));
    let state = raw.device.get("State").and_then(as_u32).unwrap_or(0);
    let address = match kind {
        DeviceKind::Wired => raw.wired.get("HwAddress"),
        DeviceKind::Wifi => raw.wireless.get("HwAddress"),
        DeviceKind::Other => None,
    }
    .and_then(as_str)
    .unwrap_or_default();
    let carrier = raw.wired.get("Carrier").and_then(as_bool).unwrap_or(false);
    let connected = state == DEVICE_STATE_ACTIVATED;
    Some(NetworkDevice {
        path: raw.path.clone(),
        name,
        kind,
        address,
        connected,
        has_link: match kind {
            DeviceKind::Wired => carrier || connected,
            _ => connected,
        },
    })
}

fn parse_networks(raw: &RawDevice, known: &Known) -> Vec<WifiNetwork> {
    let active = raw.wireless.get("ActiveAccessPoint").and_then(as_str).unwrap_or_default();
    let mut best: HashMap<String, WifiNetwork> = HashMap::new();
    let mut active_ssid = String::new();
    for ap in &raw.access_points {
        let ssid = ap.props.get("Ssid").and_then(as_bytes).map(|b| String::from_utf8_lossy(&b).into_owned());
        let Some(ssid) = ssid.filter(|s| !s.is_empty()) else { continue };
        if ap.path == active && active != "/" {
            active_ssid = ssid.clone();
        }
        let flag = |key: &str| ap.props.get(key).and_then(as_u32).unwrap_or(0);
        let strength = f64::from(flag("Strength").min(100)) / 100.0;
        let network = WifiNetwork {
            path: ap.path.clone(),
            device: raw.path.clone(),
            name: ssid.clone(),
            signal_strength: strength,
            security: security_from_flags(flag("Flags"), flag("WpaFlags"), flag("RsnFlags")),
            connected: false,
            known: known.contains_key(&ssid),
        };
        match best.get(&ssid) {
            Some(existing) if existing.signal_strength >= strength => {}
            _ => {
                best.insert(ssid, network);
            }
        }
    }
    let mut networks: Vec<WifiNetwork> = best.into_values().collect();
    for network in &mut networks {
        network.connected = !active_ssid.is_empty() && network.name == active_ssid;
    }
    networks
}

/// Builds a snapshot from raw NetworkManager data.
pub fn build_snapshot(wifi_enabled: bool, raws: &[RawDevice], profiles: &Profiles) -> NetworkSnapshot {
    let mut devices: Vec<NetworkDevice> = raws.iter().filter_map(parse_device).collect();
    devices.sort_by(|a, b| a.path.cmp(&b.path));
    let mut networks: Vec<WifiNetwork> = raws
        .iter()
        .filter(|raw| device_kind(raw.device.get("DeviceType").and_then(as_u32).unwrap_or(0)) == DeviceKind::Wifi)
        .flat_map(|raw| parse_networks(raw, &profiles.known))
        .collect();
    networks.sort_by(|a, b| {
        b.connected
            .cmp(&a.connected)
            .then(b.signal_strength.total_cmp(&a.signal_strength))
            .then_with(|| a.name.cmp(&b.name))
            .then_with(|| a.device.cmp(&b.device))
    });
    let mut vpns: Vec<VpnConnection> = profiles
        .vpns
        .iter()
        .map(|v| VpnConnection { path: v.path.clone(), name: v.name.clone(), active: v.active_path.is_some() })
        .collect();
    vpns.sort_by(|a, b| a.name.cmp(&b.name).then_with(|| a.path.cmp(&b.path)));
    NetworkSnapshot { available: true, wifi_enabled, devices, networks, vpns }
}

fn cert_blob(path: &str) -> Vec<u8> {
    let uri = if path.starts_with("file://") { path.to_owned() } else { format!("file://{path}") };
    let mut bytes = uri.into_bytes();
    bytes.push(0);
    bytes
}

fn enterprise_settings(enterprise: &Enterprise) -> HashMap<String, OwnedValue> {
    let owned = |value: ZValue<'_>| OwnedValue::try_from(value).expect("plain values convert");
    let mut eap = HashMap::from([
        ("eap".to_owned(), owned(ZValue::from(vec![enterprise.method.name().to_owned()]))),
        ("identity".to_owned(), owned(ZValue::from(enterprise.identity.as_str()))),
    ]);
    let text = |eap: &mut HashMap<String, OwnedValue>, key: &str, value: &Option<String>| {
        if let Some(value) = value {
            eap.insert(key.to_owned(), owned(ZValue::from(value.as_str())));
        }
    };
    let blob = |eap: &mut HashMap<String, OwnedValue>, key: &str, value: &Option<String>| {
        if let Some(value) = value {
            eap.insert(key.to_owned(), owned(ZValue::from(cert_blob(value))));
        }
    };
    text(&mut eap, "anonymous-identity", &enterprise.anonymous_identity);
    text(&mut eap, "domain-suffix-match", &enterprise.domain_suffix);
    text(&mut eap, "domain-match", &enterprise.domain_match);
    text(&mut eap, "subject-match", &enterprise.subject_match);
    text(&mut eap, "ca-path", &enterprise.ca_path);
    if !enterprise.altsubject_matches.is_empty() {
        eap.insert("altsubject-matches".into(), owned(ZValue::from(enterprise.altsubject_matches.clone())));
    }
    if enterprise.system_ca_certs {
        eap.insert("system-ca-certs".into(), owned(ZValue::from(true)));
    }
    blob(&mut eap, "ca-cert", &enterprise.ca_cert);
    match enterprise.method {
        EapMethod::Pwd => text(&mut eap, "password", &enterprise.password),
        EapMethod::Sim | EapMethod::Aka | EapMethod::AkaPrime => text(&mut eap, "pin", &enterprise.pin),
        EapMethod::Fast => {
            text(&mut eap, "password", &enterprise.password);
            eap.insert("phase1-fast-provisioning".into(), owned(ZValue::from(enterprise.fast_provisioning.code())));
            text(&mut eap, "pac-file", &enterprise.pac_file);
            let phase2 = enterprise.phase2.clone().or_else(|| Some("gtc".to_owned()));
            text(&mut eap, "phase2-auth", &phase2);
        }
        EapMethod::Tls => {
            blob(&mut eap, "client-cert", &enterprise.client_cert);
            blob(&mut eap, "private-key", &enterprise.private_key);
            text(&mut eap, "private-key-password", &enterprise.private_key_password);
        }
        EapMethod::Peap | EapMethod::Ttls => {
            text(&mut eap, "password", &enterprise.password);
            let phase2 = enterprise
                .phase2
                .clone()
                .or_else(|| Some(if enterprise.method == EapMethod::Peap { "mschapv2" } else { "pap" }.to_owned()));
            text(&mut eap, "phase2-auth", &phase2);
        }
    }
    eap
}

/// The connection profile for `ssid` with the given security and credentials.
pub fn connection_settings(
    ssid: &str,
    security: WifiSecurity,
    options: &ConnectOptions,
) -> HashMap<String, HashMap<String, OwnedValue>> {
    let owned = |value: ZValue<'_>| OwnedValue::try_from(value).expect("plain values convert");
    let mut settings: HashMap<String, HashMap<String, OwnedValue>> = HashMap::new();
    settings.insert(
        "connection".into(),
        HashMap::from([
            ("type".to_owned(), owned(ZValue::from("802-11-wireless"))),
            ("id".to_owned(), owned(ZValue::from(ssid))),
        ]),
    );
    settings.insert(
        "802-11-wireless".into(),
        HashMap::from([
            ("ssid".to_owned(), owned(ZValue::from(ssid.as_bytes().to_vec()))),
            ("mode".to_owned(), owned(ZValue::from("infrastructure"))),
        ]),
    );
    if options.hidden
        && let Some(wireless) = settings.get_mut("802-11-wireless")
    {
        wireless.insert("hidden".into(), owned(ZValue::from(true)));
    }
    let psk = options.psk.as_deref();
    let mut secured = false;
    if security == WifiSecurity::Eap
        && let Some(enterprise) = &options.enterprise
    {
        settings.insert(
            "802-11-wireless-security".into(),
            HashMap::from([("key-mgmt".to_owned(), owned(ZValue::from("wpa-eap")))]),
        );
        settings.insert("802-1x".into(), enterprise_settings(enterprise));
        secured = true;
    } else if let Some(psk) = psk.filter(|_| !security.is_open()) {
        let key_mgmt = match security {
            WifiSecurity::Wep => "none",
            WifiSecurity::Wpa3Sae => "sae",
            _ => "wpa-psk",
        };
        let mut entries = HashMap::from([("key-mgmt".to_owned(), owned(ZValue::from(key_mgmt)))]);
        if security == WifiSecurity::Wep {
            entries.insert("wep-key0".into(), owned(ZValue::from(psk)));
            let key_type = if options.wep_key_type == WepKeyType::Passphrase { 2u32 } else { 1u32 };
            entries.insert("wep-key-type".into(), owned(ZValue::from(key_type)));
            let auth = if options.wep_shared { "shared" } else { "open" };
            entries.insert("auth-alg".into(), owned(ZValue::from(auth)));
        } else {
            entries.insert("psk".into(), owned(ZValue::from(psk)));
        }
        settings.insert("802-11-wireless-security".into(), entries);
        secured = true;
    }
    if secured && let Some(wireless) = settings.get_mut("802-11-wireless") {
        wireless.insert("security".into(), owned(ZValue::from("802-11-wireless-security")));
    }
    settings
}

enum Event {
    Snapshot(Box<NetworkSnapshot>),
    SecretRequested(SecretRequest),
    SecretCancelled(u64),
    CommandFailed(String),
    Unavailable,
}

/// Observable state of the network service.
#[derive(Clone)]
pub struct NetworkState {
    /// True once the first snapshot arrived.
    pub ready: Signal<bool>,
    /// True when NetworkManager is reachable.
    pub available: Signal<bool>,
    /// True when Wi-Fi is switched on.
    pub wifi_enabled: Signal<bool>,
    /// True while scanning continuously.
    pub scanning: Signal<bool>,
    /// Message of the last failure, empty when none.
    pub last_error: Signal<String>,
    /// Open requests for passwords.
    pub secret_requests: Signal<Vec<SecretRequest>>,
    /// Network devices.
    pub devices: Signal<Vec<NetworkDevice>>,
    /// Wi-Fi networks in range.
    pub networks: Signal<Vec<WifiNetwork>>,
    /// VPN profiles.
    pub vpns: Signal<Vec<VpnConnection>>,
}

impl Default for NetworkState {
    fn default() -> Self {
        Self::new()
    }
}

impl NetworkState {
    /// State with nothing known.
    pub fn new() -> Self {
        NetworkState {
            ready: Signal::new(false),
            available: Signal::new(false),
            wifi_enabled: Signal::new(false),
            scanning: Signal::new(false),
            last_error: Signal::new(String::new()),
            secret_requests: Signal::new(Vec::new()),
            devices: Signal::new(Vec::new()),
            networks: Signal::new(Vec::new()),
            vpns: Signal::new(Vec::new()),
        }
    }

    /// Copies a snapshot into the signals in one batch.
    pub fn apply(&self, snapshot: &NetworkSnapshot) {
        batch(|| {
            self.available.set(snapshot.available);
            self.wifi_enabled.set(snapshot.wifi_enabled);
            if self.devices.with_untracked(|devices| *devices != snapshot.devices) {
                self.devices.set(snapshot.devices.clone());
            }
            if self.networks.with_untracked(|networks| *networks != snapshot.networks) {
                self.networks.set(snapshot.networks.clone());
            }
            if self.vpns.with_untracked(|vpns| *vpns != snapshot.vpns) {
                self.vpns.set(snapshot.vpns.clone());
            }
            self.ready.set(true);
        });
    }

    /// The first device of this kind.
    pub fn device_of(&self, kind: DeviceKind) -> Option<NetworkDevice> {
        self.devices.with_untracked(|devices| devices.iter().find(|d| d.kind == kind).cloned())
    }

    /// The Wi-Fi device.
    pub fn wifi_device(&self) -> Option<NetworkDevice> {
        self.device_of(DeviceKind::Wifi)
    }

    /// The Ethernet device.
    pub fn ethernet_device(&self) -> Option<NetworkDevice> {
        self.device_of(DeviceKind::Wired)
    }

    /// The Wi-Fi network that is connected.
    pub fn connected_network(&self) -> Option<WifiNetwork> {
        self.networks.with_untracked(|networks| networks.iter().find(|n| n.connected).cloned())
    }
}

/// Network control through NetworkManager, including the secret agent that asks the user for passwords.
pub struct Network {
    /// Its observable state.
    pub state: NetworkState,
    service: ServiceHandle<NetworkCommand>,
}

impl Network {
    /// Connects to NetworkManager on `bus` on a helper thread; `app` names the secret agent.
    pub fn start<D: 'static>(handle: &LoopHandle<'static, D>, bus: BusKind, app: &str) -> Result<Network, IoError> {
        let app = app.to_owned();
        let state = NetworkState::new();
        let target = state.clone();
        let service = spawn_supervised(
            handle,
            "acanthus-network",
            move |event| match event {
                Event::Snapshot(snapshot) => target.apply(&snapshot),
                Event::SecretRequested(request) => target.secret_requests.update(|list| list.push(request)),
                Event::CommandFailed(message) => target.last_error.set(message),
                Event::SecretCancelled(id) => target.secret_requests.update(|list| list.retain(|r| r.id != id)),
                Event::Unavailable => target.apply(&NetworkSnapshot::default()),
            },
            move |events, commands| run(bus.clone(), app.clone(), events, commands),
        )?;
        Ok(Network { state, service })
    }

    /// Sends a command; false when the service is gone.
    pub fn command(&self, command: NetworkCommand) -> bool {
        self.service.send(command)
    }

    /// Switches Wi-Fi on or off.
    pub fn set_wifi_enabled(&self, enabled: bool) -> bool {
        self.command(NetworkCommand::SetWifiEnabled(enabled))
    }

    /// Scans once.
    pub fn scan(&self) -> bool {
        self.command(NetworkCommand::Scan)
    }

    /// Scans continuously or stops.
    pub fn set_scanning(&self, scanning: bool) -> bool {
        self.state.scanning.set(scanning);
        self.command(NetworkCommand::SetScanning(scanning))
    }

    /// Connects to a network with a saved profile or an open one.
    pub fn connect(&self, ssid: &str) -> bool {
        self.connect_with(ssid, ConnectOptions::default())
    }

    /// Connects with the given credentials.
    pub fn connect_with(&self, ssid: &str, options: ConnectOptions) -> bool {
        self.command(NetworkCommand::Connect { ssid: ssid.to_owned(), options: Box::new(options) })
    }

    /// Connects with a pre-shared key.
    pub fn connect_with_psk(&self, ssid: &str, psk: &str) -> bool {
        self.connect_with(ssid, ConnectOptions { psk: Some(psk.to_owned()), ..Default::default() })
    }

    /// Connects to a WEP network.
    pub fn connect_wep(&self, ssid: &str, key: &str, key_type: WepKeyType, shared: bool) -> bool {
        self.connect_with(
            ssid,
            ConnectOptions {
                psk: Some(key.to_owned()),
                wep_key_type: key_type,
                wep_shared: shared,
                ..Default::default()
            },
        )
    }

    /// Connects to an enterprise network with identity and password.
    pub fn connect_enterprise(&self, ssid: &str, identity: &str, password: &str) -> bool {
        let enterprise =
            Enterprise { identity: identity.to_owned(), password: Some(password.to_owned()), ..Default::default() };
        self.connect_with(ssid, ConnectOptions { enterprise: Some(enterprise), ..Default::default() })
    }

    /// Connects to a hidden network, with a key when it needs one.
    pub fn connect_hidden(&self, ssid: &str, psk: Option<&str>) -> bool {
        self.connect_with(ssid, ConnectOptions { psk: psk.map(str::to_owned), hidden: true, ..Default::default() })
    }

    /// Imports a VPN or Wi-Fi profile file given as text and returns notes about what was ignored or guessed.
    pub fn import_profile(&self, name: &str, text: &str) -> Result<Vec<String>, ImportError> {
        self.import_profile_with(name, text, &ImportOptions::default())
    }

    /// Like [`Network::import_profile`] with explicit import options.
    pub fn import_profile_with(
        &self,
        name: &str,
        text: &str,
        options: &ImportOptions,
    ) -> Result<Vec<String>, ImportError> {
        let profiles = import_profiles(name, text, options)?;
        let mut notes = Vec::new();
        for profile in profiles {
            notes.extend(profile.ignored.iter().map(|i| format!("ignored {}: {i}", profile.name)));
            notes.extend(profile.guessed.iter().map(|g| format!("guessed type of {}: {g}", profile.name)));
            self.command(NetworkCommand::AddConnection(Box::new(profile.settings)));
        }
        Ok(notes)
    }

    /// Imports a profile from a file, named after the file.
    pub fn import_profile_file(&self, path: &std::path::Path) -> Result<Vec<String>, ImportError> {
        let text = std::fs::read_to_string(path)?;
        let name = path.file_stem().and_then(|s| s.to_str()).unwrap_or("imported");
        let options = ImportOptions { base_dir: path.parent().map(std::path::Path::to_path_buf), ..Default::default() };
        self.import_profile_with(name, &text, &options)
    }

    /// Connects the VPN at `path`.
    pub fn connect_vpn(&self, path: &str) -> bool {
        self.command(NetworkCommand::ConnectVpn(path.to_owned()))
    }

    /// Answers a password request with a single secret.
    pub fn provide_secret(&self, id: u64, secret: &str) -> bool {
        let key =
            self.state.secret_requests.with_untracked(|list| list.iter().find(|r| r.id == id).map(|r| r.key.clone()));
        let Some(key) = key else { return false };
        self.provide_secrets(id, HashMap::from([(key, secret.to_owned())]))
    }

    /// Answers a password request with several secrets.
    pub fn provide_secrets(&self, id: u64, secrets: HashMap<String, String>) -> bool {
        self.state.secret_requests.update(|list| list.retain(|r| r.id != id));
        self.command(NetworkCommand::ProvideSecret { id, secrets: Some(secrets) })
    }

    /// Cancels a password request.
    pub fn cancel_secret(&self, id: u64) -> bool {
        self.state.secret_requests.update(|list| list.retain(|r| r.id != id));
        self.command(NetworkCommand::ProvideSecret { id, secrets: None })
    }

    /// Disconnects the VPN at `path`.
    pub fn disconnect_vpn(&self, path: &str) -> bool {
        self.command(NetworkCommand::DisconnectVpn(path.to_owned()))
    }

    /// Disconnects the device at `device_path`.
    pub fn disconnect(&self, device_path: &str) -> bool {
        self.command(NetworkCommand::Disconnect(device_path.to_owned()))
    }

    /// Activates the best saved connection of the device at `device_path`.
    pub fn activate(&self, device_path: &str) -> bool {
        self.command(NetworkCommand::Activate(device_path.to_owned()))
    }

    /// Deletes the saved profiles of the network `ssid`.
    pub fn forget(&self, ssid: &str) -> bool {
        self.command(NetworkCommand::Forget(ssid.to_owned()))
    }
}

async fn get_all(connection: &Connection, path: &str, iface: &str) -> Properties {
    let result: zbus::Result<Properties> = async {
        let properties =
            zbus::fdo::PropertiesProxy::builder(connection).destination(SERVICE).path(path.to_owned()).build().await?;
        Ok(properties.get_all(iface.try_into()?).await?)
    }
    .await;
    result.unwrap_or_default()
}

async fn proxy(connection: &Connection, path: &str, iface: &str) -> zbus::Result<zbus::Proxy<'static>> {
    zbus::Proxy::new(connection, SERVICE, path.to_owned(), iface.to_owned()).await
}

async fn object_paths(connection: &Connection, path: &str, iface: &str, method: &str) -> zbus::Result<Vec<String>> {
    let paths: Vec<zbus::OwnedObjectPath> = proxy(connection, path, iface).await?.call(method, &()).await?;
    Ok(paths.into_iter().map(|p| p.to_string()).collect())
}

async fn read_profiles(connection: &Connection) -> Profiles {
    let mut profiles = Profiles::default();
    let Ok(paths) = object_paths(connection, SETTINGS_PATH, SETTINGS_IFACE, "ListConnections").await else {
        return profiles;
    };
    let active = active_connections(connection).await;
    for path in paths {
        let Ok(settings_proxy) = proxy(connection, &path, CONNECTION_IFACE).await else { continue };
        let reply: zbus::Result<HashMap<String, Properties>> = settings_proxy.call("GetSettings", &()).await;
        let Ok(settings) = reply else { continue };
        let kind = settings.get("connection").and_then(|c| c.get("type")).and_then(as_str);
        match kind.as_deref() {
            Some("802-11-wireless") => {
                let ssid = settings.get("802-11-wireless").and_then(|w| w.get("ssid")).and_then(as_bytes);
                if let Some(ssid) = ssid {
                    profiles.known.entry(String::from_utf8_lossy(&ssid).into_owned()).or_default().push(path);
                }
            }
            Some("vpn" | "wireguard") => {
                let name = settings.get("connection").and_then(|c| c.get("id")).and_then(as_str).unwrap_or_default();
                let active_path = active.get(&path).cloned();
                profiles.vpns.push(VpnProfile { path, name, active_path });
            }
            _ => {}
        }
    }
    profiles
}

async fn active_connections(connection: &Connection) -> HashMap<String, String> {
    let mut active = HashMap::new();
    let manager = get_all(connection, ROOT_PATH, MANAGER_IFACE).await;
    let Some(list) =
        manager.get("ActiveConnections").and_then(|v| Vec::<zbus::OwnedObjectPath>::try_from(v.clone()).ok())
    else {
        return active;
    };
    for entry in list {
        let props = get_all(connection, entry.as_str(), ACTIVE_IFACE).await;
        if let Some(profile) = props.get("Connection").and_then(as_str) {
            active.insert(profile, entry.to_string());
        }
    }
    active
}

async fn read_snapshot(connection: &Connection) -> zbus::Result<(NetworkSnapshot, Profiles)> {
    let manager = zbus::fdo::PropertiesProxy::builder(connection)
        .destination(SERVICE)
        .path(ROOT_PATH)
        .build()
        .await?
        .get_all(MANAGER_IFACE.try_into()?)
        .await?;
    let wifi_enabled = manager.get("WirelessEnabled").and_then(as_bool).unwrap_or(false);
    let device_paths = object_paths(connection, ROOT_PATH, MANAGER_IFACE, "GetDevices").await?;
    let mut raws = Vec::new();
    for path in device_paths {
        let device = get_all(connection, &path, DEVICE_IFACE).await;
        let kind = device_kind(device.get("DeviceType").and_then(as_u32).unwrap_or(0));
        let mut raw = RawDevice { path: path.clone(), device, ..Default::default() };
        match kind {
            DeviceKind::Wired => raw.wired = get_all(connection, &path, WIRED_IFACE).await,
            DeviceKind::Wifi => {
                raw.wireless = get_all(connection, &path, WIRELESS_IFACE).await;
                let aps = object_paths(connection, &path, WIRELESS_IFACE, "GetAllAccessPoints").await;
                for ap_path in aps.unwrap_or_default() {
                    let props = get_all(connection, &ap_path, AP_IFACE).await;
                    raw.access_points.push(RawAccessPoint { path: ap_path, props });
                }
            }
            DeviceKind::Other => {}
        }
        raws.push(raw);
    }
    let profiles = read_profiles(connection).await;
    Ok((build_snapshot(wifi_enabled, &raws, &profiles), profiles))
}

async fn request_scan(connection: &Connection, device: &str) -> zbus::Result<()> {
    let options: HashMap<String, ZValue<'_>> = HashMap::new();
    proxy(connection, device, WIRELESS_IFACE).await?.call::<_, _, ()>("RequestScan", &(options,)).await?;
    Ok(())
}

async fn run_command(
    connection: &Connection,
    snapshot: &NetworkSnapshot,
    profiles: &Profiles,
    pending: &Pending,
    events: &EventSink<Event>,
    command: NetworkCommand,
) {
    let wifi_devices: Vec<String> =
        snapshot.devices.iter().filter(|d| d.kind == DeviceKind::Wifi).map(|d| d.path.clone()).collect();
    let result: zbus::Result<()> = async {
        match &command {
            NetworkCommand::SetWifiEnabled(on) => {
                let properties = zbus::fdo::PropertiesProxy::builder(connection)
                    .destination(SERVICE)
                    .path(ROOT_PATH)
                    .build()
                    .await?;
                properties.set(MANAGER_IFACE.try_into()?, "WirelessEnabled", &ZValue::from(*on)).await?;
            }
            NetworkCommand::Scan | NetworkCommand::SetScanning(true) => {
                for device in &wifi_devices {
                    request_scan(connection, device).await?;
                }
            }
            NetworkCommand::SetScanning(false) => {}
            NetworkCommand::ProvideSecret { id, secrets } => {
                let sender = pending.lock().expect("pending lock").remove(id);
                if let Some(sender) = sender {
                    let _ = sender.send(secrets.clone()).await;
                }
            }
            NetworkCommand::Connect { ssid, options } => {
                let manager = proxy(connection, ROOT_PATH, MANAGER_IFACE).await?;
                let network = snapshot.networks.iter().find(|n| &n.name == ssid);
                let (device, ap, security) = match network {
                    Some(n) => (n.device.clone(), n.path.clone(), n.security),
                    None if options.hidden => {
                        let Some(device) = wifi_devices.first() else { return Ok(()) };
                        let security = match (&options.enterprise, &options.psk) {
                            (Some(_), _) => WifiSecurity::Eap,
                            (None, Some(_)) => WifiSecurity::Wpa2Psk,
                            (None, None) => WifiSecurity::Open,
                        };
                        (device.clone(), "/".to_owned(), security)
                    }
                    None => return Ok(()),
                };
                let device_path = zbus::ObjectPath::try_from(device.as_str())?;
                let ap_path = zbus::ObjectPath::try_from(ap.as_str())?;
                let saved = profiles.known.get(ssid).and_then(|paths| paths.first());
                let has_credentials = options.psk.is_some() || options.enterprise.is_some();
                match saved.filter(|_| !has_credentials) {
                    Some(saved) => {
                        let saved = zbus::ObjectPath::try_from(saved.as_str())?;
                        manager
                            .call::<_, _, zbus::OwnedObjectPath>("ActivateConnection", &(saved, device_path, ap_path))
                            .await?;
                    }
                    None => {
                        let settings = connection_settings(ssid, security, options);
                        manager
                            .call::<_, _, (zbus::OwnedObjectPath, zbus::OwnedObjectPath)>(
                                "AddAndActivateConnection",
                                &(settings, device_path, ap_path),
                            )
                            .await?;
                    }
                }
            }
            NetworkCommand::Disconnect(device) => {
                proxy(connection, device, DEVICE_IFACE).await?.call::<_, _, ()>("Disconnect", &()).await?;
            }
            NetworkCommand::Activate(device) => {
                let manager = proxy(connection, ROOT_PATH, MANAGER_IFACE).await?;
                let device = zbus::ObjectPath::try_from(device.as_str())?;
                let none = zbus::ObjectPath::try_from("/")?;
                manager
                    .call::<_, _, zbus::OwnedObjectPath>("ActivateConnection", &(none.clone(), device, none))
                    .await?;
            }
            NetworkCommand::Forget(ssid) => {
                for path in profiles.known.get(ssid).into_iter().flatten() {
                    proxy(connection, path, CONNECTION_IFACE).await?.call::<_, _, ()>("Delete", &()).await?;
                }
            }
            NetworkCommand::AddConnection(settings) => {
                proxy(connection, SETTINGS_PATH, SETTINGS_IFACE)
                    .await?
                    .call::<_, _, zbus::OwnedObjectPath>("AddConnection", &(settings.as_ref(),))
                    .await?;
            }
            NetworkCommand::ConnectVpn(path) => {
                let manager = proxy(connection, ROOT_PATH, MANAGER_IFACE).await?;
                let profile = zbus::ObjectPath::try_from(path.as_str())?;
                let none = zbus::ObjectPath::try_from("/")?;
                manager
                    .call::<_, _, zbus::OwnedObjectPath>("ActivateConnection", &(profile, none.clone(), none))
                    .await?;
            }
            NetworkCommand::DisconnectVpn(path) => {
                let active = profiles.vpns.iter().find(|v| &v.path == path).and_then(|v| v.active_path.clone());
                if let Some(active) = active {
                    let active = zbus::ObjectPath::try_from(active.as_str())?;
                    proxy(connection, ROOT_PATH, MANAGER_IFACE)
                        .await?
                        .call::<_, _, ()>("DeactivateConnection", &(active,))
                        .await?;
                }
            }
        }
        Ok(())
    }
    .await;
    if let Err(e) = result {
        tracing::debug!("network command {:?} failed: {e}", command);
        events.send(Event::CommandFailed(e.to_string()));
    }
}

type Pending = Arc<Mutex<HashMap<u64, async_channel::Sender<Option<HashMap<String, String>>>>>>;

const VPN_MESSAGE_PREFIX: &str = "x-vpn-message:";

/// The secret keys to ask for a setting: the hints, else the usual key of the setting.
pub fn secret_keys(setting: &str, hints: &[String], current: &HashMap<String, Properties>) -> Vec<String> {
    let mut keys: Vec<String> = hints.iter().filter(|h| !h.starts_with(VPN_MESSAGE_PREFIX)).cloned().collect();
    if keys.is_empty() {
        keys.push(match setting {
            "802-11-wireless-security" => {
                let wep =
                    current.get(setting).and_then(|s| s.get("key-mgmt")).and_then(as_str).is_some_and(|k| k == "none");
                if wep { "wep-key0" } else { "psk" }.to_owned()
            }
            _ => "password".to_owned(),
        });
    }
    keys
}

/// The message NetworkManager sends with a VPN secret request.
pub fn secret_message(hints: &[String]) -> String {
    hints.iter().find_map(|h| h.strip_prefix(VPN_MESSAGE_PREFIX)).map(|m| m.trim().to_owned()).unwrap_or_default()
}

/// The settings object that carries `secrets` for `setting`.
pub fn secret_settings(
    setting: &str,
    secrets: &HashMap<String, String>,
) -> HashMap<String, HashMap<String, OwnedValue>> {
    let owned = |value: ZValue<'_>| OwnedValue::try_from(value).expect("plain values convert");
    let entries: HashMap<String, OwnedValue> = if setting == "vpn" {
        let map: HashMap<String, String> = secrets.clone();
        HashMap::from([("secrets".to_owned(), owned(ZValue::from(zbus::Dict::from(map))))])
    } else {
        secrets.iter().map(|(k, v)| (k.clone(), owned(ZValue::from(v.as_str())))).collect()
    };
    HashMap::from([(setting.to_owned(), entries)])
}

struct SecretAgent {
    events: EventSink<Event>,
    pending: Pending,
    next_id: u64,
}

#[zbus::interface(name = "org.freedesktop.NetworkManager.SecretAgent")]
impl SecretAgent {
    async fn get_secrets(
        &mut self,
        connection: HashMap<String, Properties>,
        connection_path: zbus::OwnedObjectPath,
        setting_name: String,
        hints: Vec<String>,
        flags: u32,
    ) -> zbus::fdo::Result<HashMap<String, HashMap<String, OwnedValue>>> {
        if flags & SECRET_FLAG_ALLOW_INTERACTION == 0 {
            return Err(zbus::fdo::Error::Failed("no secrets without user interaction".into()));
        }
        self.next_id += 1;
        let id = self.next_id;
        let (tx, rx) = async_channel::bounded(1);
        self.pending.lock().expect("pending lock").insert(id, tx);
        let keys = secret_keys(&setting_name, &hints, &connection);
        let name = connection.get("connection").and_then(|c| c.get("id")).and_then(as_str).unwrap_or_default();
        self.events.send(Event::SecretRequested(SecretRequest {
            id,
            connection_path: connection_path.to_string(),
            connection_name: name,
            setting: setting_name.clone(),
            message: secret_message(&hints),
            key: keys[0].clone(),
            keys,
            hints: hints.clone(),
            allow_interaction: true,
            request_new: flags & SECRET_FLAG_REQUEST_NEW != 0,
        }));
        let answer = rx.recv().await.ok().flatten();
        self.pending.lock().expect("pending lock").remove(&id);
        match answer {
            Some(secrets) => Ok(secret_settings(&setting_name, &secrets)),
            None => Err(zbus::fdo::Error::Failed("user canceled".into())),
        }
    }

    async fn cancel_get_secrets(&mut self, _connection_path: zbus::OwnedObjectPath, _setting_name: String) {
        let ids: Vec<u64> = self.pending.lock().expect("pending lock").keys().copied().collect();
        for id in ids {
            if let Some(tx) = self.pending.lock().expect("pending lock").remove(&id) {
                let _ = tx.try_send(None);
                self.events.send(Event::SecretCancelled(id));
            }
        }
    }

    async fn save_secrets(&self, _connection: HashMap<String, Properties>, _connection_path: zbus::OwnedObjectPath) {}

    async fn delete_secrets(&self, _connection: HashMap<String, Properties>, _connection_path: zbus::OwnedObjectPath) {}
}

async fn register_agent(
    connection: &Connection,
    app: &str,
    events: EventSink<Event>,
    pending: Pending,
) -> zbus::Result<()> {
    connection.object_server().at(AGENT_PATH, SecretAgent { events, pending, next_id: 0 }).await?;
    proxy(connection, AGENT_MANAGER_PATH, AGENT_MANAGER_IFACE)
        .await?
        .call::<_, _, ()>("Register", &(agent_identifier(app),))
        .await
}

enum Item {
    Tick,
    Changed,
    Command(NetworkCommand),
}

async fn run(bus: BusKind, app: String, events: EventSink<Event>, commands: async_channel::Receiver<NetworkCommand>) {
    let result: zbus::Result<()> = async {
        let connection = bus.connect_shared().await?;
        let rule = zbus::MatchRule::builder()
            .msg_type(zbus::message::Type::Signal)
            .sender(SERVICE)
            .path_namespace(ROOT_PATH)
            .build()?;
        let changes =
            zbus::MessageStream::for_match_rule(rule, &connection, None).await?.map(|m| m.map(|_| Item::Changed));
        let cmds = commands.map(|c| Ok(Item::Command(c)));
        let ticks = async_io::Timer::interval(SCAN_INTERVAL).map(|_| Ok(Item::Tick));
        let mut stream = std::pin::pin!(changes.or(cmds).or(ticks));

        let pending: Pending = Arc::default();
        if let Err(e) = register_agent(&connection, &app, events.clone(), pending.clone()).await {
            tracing::info!("network secret agent not registered: {e}");
        }
        let mut scanning = false;

        let (mut current, mut profiles) = read_snapshot(&connection).await?;
        events.send(Event::Snapshot(Box::new(current.clone())));
        while let Some(item) = stream.next().await {
            let Ok(item) = item else { continue };
            match item {
                Item::Command(command) => {
                    if let NetworkCommand::SetScanning(on) = &command {
                        scanning = *on;
                    }
                    run_command(&connection, &current, &profiles, &pending, &events, command).await;
                    continue;
                }
                Item::Tick => {
                    if scanning && current.wifi_enabled {
                        run_command(&connection, &current, &profiles, &pending, &events, NetworkCommand::Scan).await;
                    }
                    continue;
                }
                Item::Changed => {}
            }
            async_io::Timer::after(COALESCE).await;
            while let Some(Some(Ok(extra))) = futures_lite::future::poll_once(stream.next()).await {
                if let Item::Command(command) = extra {
                    run_command(&connection, &current, &profiles, &pending, &events, command).await;
                }
            }
            match read_snapshot(&connection).await {
                Ok((next, next_profiles)) => {
                    profiles = next_profiles;
                    if next != current {
                        current = next;
                        events.send(Event::Snapshot(Box::new(current.clone())));
                    }
                }
                Err(e) => tracing::debug!("network refresh failed: {e}"),
            }
        }
        Ok(())
    }
    .await;
    if let Err(e) = result {
        tracing::info!("network unavailable: {e}");
        events.send(Event::Unavailable);
    }
}

#[cfg(test)]
mod tests {
    use std::sync::mpsc;
    use std::time::Instant;

    use calloop::EventLoop;
    use zbus::interface;
    use zbus::object_server::SignalEmitter;

    use super::*;
    use crate::platform::dbus::testbus::TestBus;

    fn props(entries: &[(&str, ZValue<'static>)]) -> Properties {
        entries.iter().map(|(k, v)| ((*k).to_owned(), OwnedValue::try_from(v.clone()).unwrap())).collect()
    }

    fn access_point(path: &str, ssid: &str, strength: u8, flags: u32, wpa: u32, rsn: u32) -> RawAccessPoint {
        RawAccessPoint {
            path: path.into(),
            props: props(&[
                ("Ssid", ZValue::from(ssid.as_bytes().to_vec())),
                ("Strength", ZValue::from(strength)),
                ("Flags", ZValue::from(flags)),
                ("WpaFlags", ZValue::from(wpa)),
                ("RsnFlags", ZValue::from(rsn)),
            ]),
        }
    }

    fn sample_devices() -> Vec<RawDevice> {
        vec![
            RawDevice {
                path: "/org/freedesktop/NetworkManager/Devices/1".into(),
                device: props(&[
                    ("Interface", ZValue::from("lo")),
                    ("DeviceType", ZValue::from(32u32)),
                    ("State", ZValue::from(100u32)),
                ]),
                ..Default::default()
            },
            RawDevice {
                path: "/org/freedesktop/NetworkManager/Devices/2".into(),
                device: props(&[
                    ("Interface", ZValue::from("enp3s0")),
                    ("DeviceType", ZValue::from(1u32)),
                    ("State", ZValue::from(30u32)),
                ]),
                wired: props(&[("HwAddress", ZValue::from("AA:AA")), ("Carrier", ZValue::from(true))]),
                ..Default::default()
            },
            RawDevice {
                path: "/org/freedesktop/NetworkManager/Devices/3".into(),
                device: props(&[
                    ("Interface", ZValue::from("wlan0")),
                    ("DeviceType", ZValue::from(2u32)),
                    ("State", ZValue::from(100u32)),
                ]),
                wireless: props(&[
                    ("HwAddress", ZValue::from("BB:BB")),
                    ("ActiveAccessPoint", ZValue::from(zbus::ObjectPath::from_static_str_unchecked("/ap/2"))),
                ]),
                access_points: vec![
                    access_point("/ap/1", "Cafe", 40, 0, 0, 0),
                    access_point("/ap/2", "Home", 70, 1, 0, KEY_MGMT_PSK),
                    access_point("/ap/3", "Home", 90, 1, 0, KEY_MGMT_PSK),
                    access_point("/ap/4", "", 99, 0, 0, 0),
                    access_point("/ap/5", "Work", 55, 1, 0, KEY_MGMT_8021X),
                ],
                ..Default::default()
            },
        ]
    }

    #[test]
    fn security_flags_map_to_modes() {
        assert_eq!(security_from_flags(0, 0, 0), WifiSecurity::Open);
        assert_eq!(security_from_flags(1, 0, 0), WifiSecurity::Wep);
        assert_eq!(security_from_flags(1, KEY_MGMT_PSK, 0), WifiSecurity::WpaPsk);
        assert_eq!(security_from_flags(1, KEY_MGMT_PSK, KEY_MGMT_PSK), WifiSecurity::Wpa2Psk);
        assert_eq!(security_from_flags(1, 0, KEY_MGMT_SAE), WifiSecurity::Wpa3Sae);
        assert_eq!(security_from_flags(1, 0, KEY_MGMT_8021X), WifiSecurity::Eap);
        assert_eq!(security_from_flags(0, 0, KEY_MGMT_OWE), WifiSecurity::Owe);
        assert!(WifiSecurity::Owe.is_open() && !WifiSecurity::Wep.is_open());
    }

    #[test]
    fn devices_and_networks_are_built_from_raw_objects() {
        let profiles = Profiles {
            known: Known::from([("Home".to_owned(), vec!["/conn/1".to_owned()])]),
            vpns: vec![
                VpnProfile { path: "/conn/9".into(), name: "Work vpn".into(), active_path: Some("/active/1".into()) },
                VpnProfile { path: "/conn/8".into(), name: "Another".into(), active_path: None },
            ],
        };
        let snapshot = build_snapshot(true, &sample_devices(), &profiles);
        let vpns: Vec<(&str, bool)> = snapshot.vpns.iter().map(|v| (v.name.as_str(), v.active)).collect();
        assert_eq!(vpns, [("Another", false), ("Work vpn", true)]);
        assert!(snapshot.networks.iter().all(|n| n.device.ends_with("/Devices/3")));
        assert!(snapshot.available && snapshot.wifi_enabled);
        assert_eq!(snapshot.devices.len(), 2);
        let wired = &snapshot.devices[0];
        assert_eq!((wired.name.as_str(), wired.kind, wired.address.as_str()), ("enp3s0", DeviceKind::Wired, "AA:AA"));
        assert!(!wired.connected && wired.has_link);
        let wifi = &snapshot.devices[1];
        assert_eq!((wifi.kind, wifi.connected), (DeviceKind::Wifi, true));

        let names: Vec<&str> = snapshot.networks.iter().map(|n| n.name.as_str()).collect();
        assert_eq!(names, ["Home", "Work", "Cafe"]);
        let home = &snapshot.networks[0];
        assert!(home.connected && home.known);
        assert_eq!(home.signal_strength, 0.9);
        assert_eq!(home.path, "/ap/3");
        assert_eq!(home.security, WifiSecurity::Wpa2Psk);
        assert_eq!(snapshot.networks[1].security, WifiSecurity::Eap);
        assert!(!snapshot.networks[2].known && snapshot.networks[2].security.is_open());
    }

    #[test]
    fn connection_settings_carry_ssid_and_key() {
        let open = connection_settings("Cafe", WifiSecurity::Open, &psk_options("ignored"));
        assert!(!open.contains_key("802-11-wireless-security"));
        let secured = connection_settings("Home", WifiSecurity::Wpa2Psk, &psk_options("hunter22"));
        let security = &secured["802-11-wireless-security"];
        assert_eq!(as_str(&security["key-mgmt"]).as_deref(), Some("wpa-psk"));
        assert_eq!(as_str(&security["psk"]).as_deref(), Some("hunter22"));
        assert_eq!(as_bytes(&secured["802-11-wireless"]["ssid"]).unwrap(), b"Home");
        let sae = connection_settings("Home", WifiSecurity::Wpa3Sae, &psk_options("pw"));
        assert_eq!(as_str(&sae["802-11-wireless-security"]["key-mgmt"]).as_deref(), Some("sae"));
    }

    fn psk_options(psk: &str) -> ConnectOptions {
        ConnectOptions { psk: Some(psk.to_owned()), ..Default::default() }
    }

    #[test]
    fn hidden_and_peap_profiles_are_described() {
        let hidden = ConnectOptions { hidden: true, ..psk_options("pw") };
        let hidden = connection_settings("Ghost", WifiSecurity::Wpa2Psk, &hidden);
        assert_eq!(as_bool(&hidden["802-11-wireless"]["hidden"]), Some(true));
        let enterprise = Enterprise {
            identity: "alice".into(),
            password: Some("secret".into()),
            anonymous_identity: Some("anon".into()),
            ca_cert: Some("/etc/ca.pem".into()),
            ..Default::default()
        };
        let options = ConnectOptions { enterprise: Some(enterprise), ..Default::default() };
        let eap = connection_settings("Work", WifiSecurity::Eap, &options);
        assert_eq!(as_str(&eap["802-11-wireless-security"]["key-mgmt"]).as_deref(), Some("wpa-eap"));
        let x = &eap["802-1x"];
        assert_eq!(as_str(&x["identity"]).as_deref(), Some("alice"));
        assert_eq!(as_str(&x["password"]).as_deref(), Some("secret"));
        assert_eq!(as_str(&x["phase2-auth"]).as_deref(), Some("mschapv2"));
        assert_eq!(as_str(&x["anonymous-identity"]).as_deref(), Some("anon"));
        assert_eq!(as_bytes(&x["ca-cert"]).unwrap(), b"file:///etc/ca.pem\0");
        assert_eq!(as_str(&eap["802-11-wireless"]["security"]).as_deref(), Some("802-11-wireless-security"));
    }

    #[test]
    fn tls_profiles_carry_certificates_and_key() {
        let enterprise = Enterprise {
            method: EapMethod::Tls,
            identity: "alice".into(),
            password: Some("ignored".into()),
            client_cert: Some("/home/a/client.pem".into()),
            private_key: Some("file:///home/a/key.pem".into()),
            private_key_password: Some("keypw".into()),
            ..Default::default()
        };
        let options = ConnectOptions { enterprise: Some(enterprise), ..Default::default() };
        let eap = connection_settings("Corp", WifiSecurity::Eap, &options);
        let x = &eap["802-1x"];
        assert_eq!(as_bytes(&x["client-cert"]).unwrap(), b"file:///home/a/client.pem\0");
        assert_eq!(as_bytes(&x["private-key"]).unwrap(), b"file:///home/a/key.pem\0");
        assert_eq!(as_str(&x["private-key-password"]).as_deref(), Some("keypw"));
        assert!(!x.contains_key("password") && !x.contains_key("phase2-auth"));
        assert_eq!(Vec::<String>::try_from(x["eap"].clone()).unwrap(), ["tls"]);
    }

    fn eap_settings(enterprise: Enterprise) -> HashMap<String, OwnedValue> {
        let options = ConnectOptions { enterprise: Some(enterprise), ..Default::default() };
        connection_settings("Corp", WifiSecurity::Eap, &options).remove("802-1x").unwrap()
    }

    #[test]
    fn fast_pwd_and_sim_profiles_are_described() {
        let fast = eap_settings(Enterprise {
            method: EapMethod::Fast,
            identity: "alice".into(),
            password: Some("pw".into()),
            fast_provisioning: FastProvisioning::Authenticated,
            pac_file: Some("/home/a/pac".into()),
            ..Default::default()
        });
        assert_eq!(Vec::<String>::try_from(fast["eap"].clone()).unwrap(), ["fast"]);
        assert_eq!(as_str(&fast["phase1-fast-provisioning"]).as_deref(), Some("2"));
        assert_eq!(as_str(&fast["pac-file"]).as_deref(), Some("/home/a/pac"));
        assert_eq!(as_str(&fast["phase2-auth"]).as_deref(), Some("gtc"));
        assert_eq!(as_str(&fast["password"]).as_deref(), Some("pw"));

        let pwd = eap_settings(Enterprise {
            method: EapMethod::Pwd,
            identity: "bob".into(),
            password: Some("pw".into()),
            ..Default::default()
        });
        assert_eq!(Vec::<String>::try_from(pwd["eap"].clone()).unwrap(), ["pwd"]);
        assert_eq!(as_str(&pwd["password"]).as_deref(), Some("pw"));
        assert!(!pwd.contains_key("phase2-auth"));

        let sim = eap_settings(Enterprise {
            method: EapMethod::Sim,
            identity: "1234567890@wlan.example".into(),
            pin: Some("0000".into()),
            password: Some("ignored".into()),
            ..Default::default()
        });
        assert_eq!(Vec::<String>::try_from(sim["eap"].clone()).unwrap(), ["sim"]);
        assert_eq!(as_str(&sim["pin"]).as_deref(), Some("0000"));
        assert!(!sim.contains_key("password"));
    }

    #[test]
    fn aka_profiles_use_identity_and_pin() {
        for (method, name) in [(EapMethod::Aka, "aka"), (EapMethod::AkaPrime, "aka'")] {
            let settings = eap_settings(Enterprise {
                method,
                identity: "0123@nai".into(),
                pin: Some("1234".into()),
                password: Some("ignored".into()),
                ..Default::default()
            });
            assert_eq!(Vec::<String>::try_from(settings["eap"].clone()).unwrap(), [name]);
            assert_eq!(as_str(&settings["pin"]).as_deref(), Some("1234"));
            assert!(!settings.contains_key("password"));
        }
    }

    #[test]
    fn server_pinning_fields_are_written() {
        let pinned = eap_settings(Enterprise {
            identity: "alice".into(),
            ca_cert: Some("/etc/server.pem".into()),
            domain_match: Some("radius.example.com".into()),
            subject_match: Some("/CN=radius".into()),
            altsubject_matches: vec!["DNS:radius.example.com".into()],
            ca_path: Some("/etc/certs".into()),
            system_ca_certs: true,
            ..Default::default()
        });
        assert_eq!(as_str(&pinned["domain-match"]).as_deref(), Some("radius.example.com"));
        assert_eq!(as_str(&pinned["subject-match"]).as_deref(), Some("/CN=radius"));
        assert_eq!(Vec::<String>::try_from(pinned["altsubject-matches"].clone()).unwrap(), ["DNS:radius.example.com"]);
        assert_eq!(as_str(&pinned["ca-path"]).as_deref(), Some("/etc/certs"));
        assert_eq!(as_bool(&pinned["system-ca-certs"]), Some(true));
        let bare = eap_settings(Enterprise { identity: "a".into(), ..Default::default() });
        assert!(!bare.contains_key("system-ca-certs") && !bare.contains_key("altsubject-matches"));
    }

    #[test]
    fn wep_profiles_use_key_type_and_auth() {
        let options = ConnectOptions { wep_key_type: WepKeyType::Passphrase, wep_shared: true, ..psk_options("abc") };
        let wep = connection_settings("Old", WifiSecurity::Wep, &options);
        let security = &wep["802-11-wireless-security"];
        assert_eq!(as_str(&security["key-mgmt"]).as_deref(), Some("none"));
        assert_eq!(as_str(&security["wep-key0"]).as_deref(), Some("abc"));
        assert_eq!(as_u32(&security["wep-key-type"]), Some(2));
        assert_eq!(as_str(&security["auth-alg"]).as_deref(), Some("shared"));
        let plain = connection_settings("Old", WifiSecurity::Wep, &psk_options("abc"));
        assert_eq!(as_u32(&plain["802-11-wireless-security"]["wep-key-type"]), Some(1));
        assert_eq!(as_str(&plain["802-11-wireless-security"]["auth-alg"]).as_deref(), Some("open"));
    }

    #[test]
    fn secret_replies_use_the_requested_keys() {
        let none = HashMap::new();
        assert_eq!(secret_keys("802-11-wireless-security", &[], &none), ["psk"]);
        let wep =
            HashMap::from([("802-11-wireless-security".to_owned(), props(&[("key-mgmt", ZValue::from("none"))]))]);
        assert_eq!(secret_keys("802-11-wireless-security", &[], &wep), ["wep-key0"]);
        assert_eq!(secret_keys("802-1x", &[], &none), ["password"]);
        let hints = vec!["x-vpn-message: Enter your token".to_owned(), "password".to_owned(), "2fa".to_owned()];
        assert_eq!(secret_keys("vpn", &hints, &none), ["password", "2fa"]);
        assert_eq!(secret_message(&hints), "Enter your token");

        let plain = secret_settings("802-1x", &HashMap::from([("password".to_owned(), "x".to_owned())]));
        assert_eq!(as_str(&plain["802-1x"]["password"]).as_deref(), Some("x"));
        let secrets = HashMap::from([("password".to_owned(), "p".to_owned()), ("2fa".to_owned(), "123".to_owned())]);
        let vpn = secret_settings("vpn", &secrets);
        let dict = HashMap::<String, String>::try_from(vpn["vpn"]["secrets"].clone()).unwrap();
        assert_eq!(dict, secrets);
    }

    #[test]
    fn state_helpers_pick_devices_and_active_network() {
        let state = NetworkState::new();
        state.apply(&build_snapshot(true, &sample_devices(), &Profiles::default()));
        assert_eq!(state.wifi_device().unwrap().name, "wlan0");
        assert_eq!(state.ethernet_device().unwrap().name, "enp3s0");
        assert_eq!(state.connected_network().unwrap().name, "Home");
    }

    struct FakeManager {
        wifi: bool,
        events: mpsc::Sender<String>,
    }

    #[interface(name = "org.freedesktop.NetworkManager")]
    impl FakeManager {
        #[zbus(property)]
        fn wireless_enabled(&self) -> bool {
            self.wifi
        }
        #[zbus(property)]
        fn set_wireless_enabled(&mut self, value: bool) {
            self.wifi = value;
        }
        fn activate_connection(
            &self,
            connection: zbus::OwnedObjectPath,
            device: zbus::OwnedObjectPath,
            specific: zbus::OwnedObjectPath,
        ) -> zbus::OwnedObjectPath {
            let _ = self.events.send(format!("activate {connection} {device} {specific}"));
            zbus::OwnedObjectPath::try_from("/org/freedesktop/NetworkManager/ActiveConnection/1").unwrap()
        }
        fn add_and_activate_connection(
            &self,
            settings: HashMap<String, Properties>,
            _device: zbus::OwnedObjectPath,
            _specific: zbus::OwnedObjectPath,
        ) -> (zbus::OwnedObjectPath, zbus::OwnedObjectPath) {
            let psk = settings
                .get("802-11-wireless-security")
                .and_then(|s| s.get("psk"))
                .and_then(as_str)
                .unwrap_or_default();
            let _ = self.events.send(format!("add {psk}"));
            let path = zbus::OwnedObjectPath::try_from("/org/freedesktop/NetworkManager/ActiveConnection/1").unwrap();
            (path.clone(), path)
        }
        fn get_devices(&self) -> Vec<zbus::OwnedObjectPath> {
            vec![zbus::OwnedObjectPath::try_from("/org/freedesktop/NetworkManager/Devices/3").unwrap()]
        }
    }

    struct FakeDevice;

    #[interface(name = "org.freedesktop.NetworkManager.Device")]
    impl FakeDevice {
        #[zbus(property)]
        fn interface(&self) -> String {
            "wlan0".into()
        }
        #[zbus(property)]
        fn device_type(&self) -> u32 {
            2
        }
        #[zbus(property)]
        fn state(&self) -> u32 {
            100
        }
    }

    struct FakeWireless {
        events: mpsc::Sender<String>,
        active: bool,
    }

    #[interface(name = "org.freedesktop.NetworkManager.Device.Wireless")]
    impl FakeWireless {
        #[zbus(property)]
        fn hw_address(&self) -> String {
            "BB:BB".into()
        }
        #[zbus(property)]
        fn active_access_point(&self) -> zbus::OwnedObjectPath {
            let path = if self.active { "/org/freedesktop/NetworkManager/AccessPoint/1" } else { "/" };
            zbus::OwnedObjectPath::try_from(path).unwrap()
        }
        fn get_all_access_points(&self) -> Vec<zbus::OwnedObjectPath> {
            vec![zbus::OwnedObjectPath::try_from("/org/freedesktop/NetworkManager/AccessPoint/1").unwrap()]
        }
        async fn request_scan(
            &mut self,
            _options: HashMap<String, zbus::OwnedVariant>,
            #[zbus(signal_emitter)] emitter: SignalEmitter<'_>,
        ) {
            self.active = true;
            let _ = self.events.send("scan".into());
            let _ = self.active_access_point_changed(&emitter).await;
        }
    }

    struct FakeAccessPoint;

    #[interface(name = "org.freedesktop.NetworkManager.AccessPoint")]
    impl FakeAccessPoint {
        #[zbus(property)]
        fn ssid(&self) -> Vec<u8> {
            b"Home".to_vec()
        }
        #[zbus(property)]
        fn strength(&self) -> u8 {
            80
        }
        #[zbus(property)]
        fn flags(&self) -> u32 {
            1
        }
        #[zbus(property)]
        fn wpa_flags(&self) -> u32 {
            0
        }
        #[zbus(property)]
        fn rsn_flags(&self) -> u32 {
            KEY_MGMT_PSK
        }
    }

    struct FakeSettings {
        events: mpsc::Sender<String>,
    }

    #[interface(name = "org.freedesktop.NetworkManager.Settings")]
    impl FakeSettings {
        fn add_connection(&self, settings: HashMap<String, Properties>) -> zbus::OwnedObjectPath {
            let id = settings.get("connection").and_then(|c| c.get("id")).and_then(as_str).unwrap_or_default();
            let _ = self.events.send(format!("added {id}"));
            zbus::OwnedObjectPath::try_from("/org/freedesktop/NetworkManager/Settings/2").unwrap()
        }
        fn list_connections(&self) -> Vec<zbus::OwnedObjectPath> {
            vec![zbus::OwnedObjectPath::try_from("/org/freedesktop/NetworkManager/Settings/1").unwrap()]
        }
    }

    struct FakeConnection {
        events: mpsc::Sender<String>,
    }

    struct FakeAgentManager {
        events: mpsc::Sender<String>,
    }

    #[interface(name = "org.freedesktop.NetworkManager.AgentManager")]
    impl FakeAgentManager {
        fn register(&self, _identifier: String, #[zbus(header)] header: zbus::message::Header<'_>) {
            let sender = header.sender().map(|s| s.to_string()).unwrap_or_default();
            let _ = self.events.send(sender);
        }
    }

    #[interface(name = "org.freedesktop.NetworkManager.Settings.Connection")]
    impl FakeConnection {
        fn delete(&self) {
            let _ = self.events.send("delete".into());
        }
        fn get_settings(&self) -> HashMap<String, Properties> {
            HashMap::from([
                ("connection".to_owned(), props(&[("type", ZValue::from("802-11-wireless"))])),
                ("802-11-wireless".to_owned(), props(&[("ssid", ZValue::from(b"Home".to_vec()))])),
            ])
        }
    }

    fn pump_until(event_loop: &mut EventLoop<'static, ()>, done: impl Fn() -> bool) -> bool {
        let deadline = Instant::now() + Duration::from_secs(10);
        while !done() && Instant::now() < deadline {
            event_loop.dispatch(Duration::from_millis(10), &mut ()).unwrap();
        }
        done()
    }

    #[test]
    fn missing_service_leaves_network_unavailable() {
        let bus = TestBus::start();
        let mut event_loop: EventLoop<'static, ()> = EventLoop::try_new().unwrap();
        let network = Network::start(&event_loop.handle(), bus.kind(), "demo").unwrap();
        assert!(pump_until(&mut event_loop, || network.state.ready.get_untracked()));
        assert!(!network.state.available.get_untracked());
        assert!(network.state.networks.get_untracked().is_empty());
    }

    #[test]
    fn service_state_flows_in_and_commands_flow_out() {
        let bus = TestBus::start();
        let (event_tx, event_rx) = mpsc::channel();
        let (manager_tx, manager_rx) = mpsc::channel();
        let (agent_tx, agent_rx) = mpsc::channel();
        let (ready_tx, ready_rx) = mpsc::channel();
        let address = bus.address.clone();
        let manager_tx_conn = manager_tx.clone();
        let manager_tx_settings = manager_tx.clone();
        std::thread::spawn(move || {
            async_io::block_on(async move {
                let _connection = zbus::connection::Builder::address(address.as_str())
                    .name(SERVICE)
                    .serve_at(ROOT_PATH, FakeManager { wifi: true, events: manager_tx })
                    .serve_at("/org/freedesktop/NetworkManager/Devices/3", FakeDevice)
                    .serve_at(
                        "/org/freedesktop/NetworkManager/Devices/3",
                        FakeWireless { events: event_tx, active: false },
                    )
                    .serve_at("/org/freedesktop/NetworkManager/AccessPoint/1", FakeAccessPoint)
                    .serve_at(SETTINGS_PATH, FakeSettings { events: manager_tx_settings })
                    .serve_at("/org/freedesktop/NetworkManager/Settings/1", FakeConnection { events: manager_tx_conn })
                    .serve_at(AGENT_MANAGER_PATH, FakeAgentManager { events: agent_tx })
                    .build()
                    .await
                    .unwrap();
                ready_tx.send(()).unwrap();
                std::future::pending::<()>().await;
            });
        });
        ready_rx.recv_timeout(Duration::from_secs(10)).unwrap();

        let mut event_loop: EventLoop<'static, ()> = EventLoop::try_new().unwrap();
        let network = Network::start(&event_loop.handle(), bus.kind(), "demo").unwrap();
        let state = network.state.clone();
        assert!(pump_until(&mut event_loop, || state.ready.get_untracked()));
        assert!(state.available.get_untracked());
        assert!(state.wifi_enabled.get_untracked());
        assert_eq!(state.wifi_device().unwrap().address, "BB:BB");
        let networks = state.networks.get_untracked();
        assert_eq!(networks.len(), 1);
        assert!(networks[0].known && !networks[0].connected);
        assert_eq!(networks[0].security, WifiSecurity::Wpa2Psk);

        assert!(network.scan());
        assert_eq!(event_rx.recv_timeout(Duration::from_secs(10)).unwrap(), "scan");
        assert!(pump_until(&mut event_loop, || state.connected_network().is_some()));

        let wait = |rx: &mpsc::Receiver<String>| rx.recv_timeout(Duration::from_secs(10)).unwrap();

        assert!(network.connect_with_psk("Home", "pw"));
        assert_eq!(wait(&manager_rx), "add pw");
        assert!(network.connect("Home"));
        assert_eq!(
            wait(&manager_rx),
            "activate /org/freedesktop/NetworkManager/Settings/1 /org/freedesktop/NetworkManager/Devices/3 \
             /org/freedesktop/NetworkManager/AccessPoint/1"
        );
        assert!(network.forget("Home"));
        assert_eq!(wait(&manager_rx), "delete");
        assert!(network.activate("/org/freedesktop/NetworkManager/Devices/3"));
        assert_eq!(wait(&manager_rx), "activate / /org/freedesktop/NetworkManager/Devices/3 /");

        let ignored = network
            .import_profile("Imported", "[connection]\nid=Imported\ntype=wifi\n\n[wifi]\nssid=Imported\n")
            .unwrap();
        assert!(ignored.is_empty());
        assert_eq!(wait(&manager_rx), "added Imported");
        assert!(network.import_profile("bad", "nothing").is_err());

        let agent_owner = wait(&agent_rx);
        let (answer_tx, answer_rx) = mpsc::channel();
        let client_bus = bus.kind();
        std::thread::spawn(move || {
            async_io::block_on(async move {
                let client = client_bus.connect().await.unwrap();
                let agent =
                    zbus::Proxy::new(&client, agent_owner, AGENT_PATH, "org.freedesktop.NetworkManager.SecretAgent")
                        .await
                        .unwrap();
                let connection: HashMap<String, Properties> =
                    HashMap::from([("connection".to_owned(), props(&[("id", ZValue::from("Home"))]))]);
                let path = zbus::ObjectPath::try_from("/org/freedesktop/NetworkManager/Settings/1").unwrap();
                let reply: zbus::Result<HashMap<String, Properties>> = agent
                    .call("GetSecrets", &(connection, path, "802-11-wireless-security", Vec::<String>::new(), 1u32))
                    .await;
                answer_tx.send(reply.map(|r| as_str(&r["802-11-wireless-security"]["psk"]).unwrap())).unwrap();
            });
        });
        assert!(pump_until(&mut event_loop, || !state.secret_requests.get_untracked().is_empty()));
        let request = state.secret_requests.get_untracked()[0].clone();
        assert_eq!((request.connection_name.as_str(), request.setting.as_str()), ("Home", "802-11-wireless-security"));
        assert!(network.provide_secret(request.id, "hunter22"));
        assert_eq!(answer_rx.recv_timeout(Duration::from_secs(10)).unwrap().unwrap(), "hunter22");
        assert!(state.secret_requests.get_untracked().is_empty());

        assert!(network.set_wifi_enabled(false));
        assert!(pump_until(&mut event_loop, || !state.wifi_enabled.get_untracked()));
    }
}
