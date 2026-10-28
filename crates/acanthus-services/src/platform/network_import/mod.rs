mod keyfile;
mod openvpn;
mod schema;
mod vpn;
mod wireguard;
mod wpa;

use std::collections::HashMap;
use std::net::{Ipv4Addr, Ipv6Addr};
use std::path::PathBuf;

use zbus::{OwnedVariant as OwnedValue, Variant as ZValue};

use crate::platform::network::ConnectionSettings;

pub use keyfile::{import_keyfile, import_keyfile_with};
pub use openvpn::import_openvpn;
pub use schema::ValueType;
pub use vpn::{import_l2tp, import_openconnect, import_pptp, import_strongswan};
pub use wireguard::import_wireguard;
pub use wpa::import_wpa_supplicant;

/// Why a profile could not be imported.
#[derive(Debug, thiserror::Error)]
pub enum ImportError {
    /// The text is in none of the supported formats.
    #[error("the file is not a NetworkManager keyfile, WireGuard, wpa_supplicant or OpenVPN configuration")]
    Unrecognised,
    /// A required entry is missing; the name says which.
    #[error("the profile has no {0}")]
    Missing(&'static str),
    /// A value cannot be used.
    #[error("invalid value {value:?} for {key}")]
    Invalid {
        /// The setting that has the bad value.
        key: String,
        /// The value found.
        value: String,
    },
    /// The profile file could not be read or written.
    #[error("could not read or write a profile file: {0}")]
    Io(#[from] std::io::Error),
}

/// A profile converted into NetworkManager connection settings, with notes about what the conversion left out.
#[derive(Debug, Clone, Default)]
pub struct ImportedProfile {
    /// Name of the connection.
    pub name: String,
    /// The settings by setting name.
    pub settings: ConnectionSettings,
    /// Entries that could not be converted.
    pub ignored: Vec<String>,
    /// Entries whose type was guessed.
    pub guessed: Vec<String>,
}

/// A supported profile format.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProfileFormat {
    /// NetworkManager keyfile.
    Keyfile,
    /// WireGuard configuration.
    WireGuard,
    /// wpa_supplicant configuration.
    WpaSupplicant,
    /// OpenVPN configuration.
    OpenVpn,
    /// OpenConnect configuration.
    OpenConnect,
    /// strongSwan configuration.
    StrongSwan,
    /// L2TP configuration.
    L2tp,
    /// PPTP configuration.
    Pptp,
}

/// Options of a profile import.
#[derive(Debug, Clone, Default)]
pub struct ImportOptions {
    /// The format, or `None` to detect it.
    pub format: Option<ProfileFormat>,
    /// Types of keys the built-in schema does not know, by `setting.key`.
    pub types: HashMap<String, ValueType>,
    /// Directory that relative file references are resolved against.
    pub base_dir: Option<PathBuf>,
    /// Directory for inline certificates and keys written next to an imported profile; required when the profile has any.
    pub output_dir: Option<PathBuf>,
}

type Group = (String, Vec<(String, String)>);

pub(crate) fn owned(value: ZValue<'_>) -> OwnedValue {
    OwnedValue::try_from(value).expect("plain values convert")
}

pub(crate) fn invalid(key: &str, value: &str) -> ImportError {
    ImportError::Invalid { key: key.to_owned(), value: value.to_owned() }
}

pub(crate) fn parse_groups(text: &str) -> Vec<Group> {
    let mut groups: Vec<Group> = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') || line.starts_with(';') {
            continue;
        }
        if let Some(name) = line.strip_prefix('[').and_then(|l| l.strip_suffix(']')) {
            groups.push((name.trim().to_owned(), Vec::new()));
        } else if let Some((key, value)) = line.split_once('=')
            && let Some(group) = groups.last_mut()
        {
            group.1.push((key.trim().to_owned(), value.trim().to_owned()));
        }
    }
    groups
}

pub(crate) fn split_list(value: &str, separator: char) -> Vec<String> {
    let mut items = Vec::new();
    let mut current = String::new();
    let mut escaped = false;
    for c in value.chars() {
        if escaped {
            current.push(c);
            escaped = false;
        } else if c == '\\' {
            escaped = true;
        } else if c == separator {
            items.push(std::mem::take(&mut current));
        } else {
            current.push(c);
        }
    }
    items.push(current);
    items.into_iter().map(|i| i.trim().to_owned()).filter(|i| !i.is_empty()).collect()
}

pub(crate) fn parse_bool(key: &str, value: &str) -> Result<bool, ImportError> {
    match value.to_ascii_lowercase().as_str() {
        "true" | "1" | "yes" | "on" => Ok(true),
        "false" | "0" | "no" | "off" => Ok(false),
        _ => Err(invalid(key, value)),
    }
}

pub(crate) fn parse_mac(key: &str, value: &str) -> Result<Vec<u8>, ImportError> {
    let bytes: Result<Vec<u8>, _> = value.split(':').map(|part| u8::from_str_radix(part, 16)).collect();
    bytes.ok().filter(|b| b.len() == 6).ok_or_else(|| invalid(key, value))
}

pub(crate) fn parse_hex(key: &str, value: &str) -> Result<Vec<u8>, ImportError> {
    let digits: String = value.chars().filter(|c| !matches!(c, ':' | ' ')).collect();
    if !digits.len().is_multiple_of(2) {
        return Err(invalid(key, value));
    }
    (0..digits.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&digits[i..i + 2], 16).map_err(|_| invalid(key, value)))
        .collect()
}

pub(crate) fn cert_value(value: &str) -> Vec<u8> {
    let mut bytes = if value.starts_with("pkcs11:") || value.starts_with("file://") {
        value.to_owned()
    } else {
        format!("file://{value}")
    }
    .into_bytes();
    bytes.push(0);
    bytes
}

pub(crate) fn dict(entries: Vec<(&str, ZValue<'static>)>) -> ZValue<'static> {
    let map: HashMap<String, ZValue<'static>> = entries.into_iter().map(|(k, v)| (k.to_owned(), v)).collect();
    ZValue::from(zbus::Dict::from(map))
}

pub(crate) fn dict_array(items: Vec<ZValue<'static>>) -> ZValue<'static> {
    let signature = zbus::Signature::try_from("a{sv}").expect("valid signature");
    let mut array = zbus::Array::new(&signature);
    for item in items {
        array.append(item).expect("dict elements");
    }
    ZValue::from(array)
}

pub(crate) fn string_map(entries: &[(String, String)]) -> ZValue<'static> {
    ZValue::from(zbus::Dict::from(entries.iter().cloned().collect::<HashMap<String, String>>()))
}

pub(crate) fn address_entry(family: &str, value: &str) -> Result<(String, u32, Option<String>), ImportError> {
    let mut parts = value.split(',');
    let address = parts.next().unwrap_or_default().trim();
    let gateway = parts.next().map(|g| g.trim().to_owned()).filter(|g| !g.is_empty());
    let (ip, prefix) = match address.split_once('/') {
        Some((ip, prefix)) => (ip, prefix.parse::<u32>().map_err(|_| invalid("address", value))?),
        None => (address, if family == "ipv4" { 32 } else { 128 }),
    };
    if !valid_ip(family, ip) {
        return Err(invalid("address", value));
    }
    Ok((ip.to_owned(), prefix, gateway))
}

pub(crate) fn valid_ip(family: &str, ip: &str) -> bool {
    if family == "ipv4" { ip.parse::<Ipv4Addr>().is_ok() } else { ip.parse::<Ipv6Addr>().is_ok() }
}

pub(crate) fn route_entry(family: &str, value: &str) -> Result<ZValue<'static>, ImportError> {
    let mut parts = value.split(',').map(str::trim);
    let destination = parts.next().unwrap_or_default();
    let (ip, prefix) = destination.split_once('/').ok_or_else(|| invalid("route", value))?;
    let prefix = prefix.parse::<u32>().map_err(|_| invalid("route", value))?;
    if !valid_ip(family, ip) {
        return Err(invalid("route", value));
    }
    let mut fields: Vec<(&str, ZValue<'static>)> =
        vec![("dest", ZValue::from(ip.to_owned())), ("prefix", ZValue::from(prefix))];
    if let Some(next_hop) = parts.next().filter(|h| !h.is_empty()) {
        if !valid_ip(family, next_hop) {
            return Err(invalid("route", value));
        }
        fields.push(("next-hop", ZValue::from(next_hop.to_owned())));
    }
    if let Some(metric) = parts.next().filter(|m| !m.is_empty()) {
        fields.push(("metric", ZValue::from(metric.parse::<u32>().map_err(|_| invalid("route", value))?)));
    }
    Ok(dict(fields))
}

pub(crate) fn dns_value(family: &str, value: &str) -> Result<OwnedValue, ImportError> {
    let servers = split_list(value, ';');
    if family == "ipv4" {
        let mut out = Vec::new();
        for server in &servers {
            let ip: Ipv4Addr = server.parse().map_err(|_| invalid("dns", server))?;
            out.push(u32::from_ne_bytes(ip.octets()));
        }
        Ok(owned(ZValue::from(out)))
    } else {
        let mut out: Vec<Vec<u8>> = Vec::new();
        for server in &servers {
            let ip: Ipv6Addr = server.parse().map_err(|_| invalid("dns", server))?;
            out.push(ip.octets().to_vec());
        }
        Ok(owned(ZValue::from(out)))
    }
}

/// Guesses the format of a profile text.
pub fn detect_format(text: &str) -> Option<ProfileFormat> {
    let lower = text.to_ascii_lowercase();
    let compact: String = lower.chars().filter(|c| !c.is_whitespace()).collect();
    if lower.contains("[interface]") {
        Some(ProfileFormat::WireGuard)
    } else if lower.contains("[connection]") {
        Some(ProfileFormat::Keyfile)
    } else if compact.contains("network={") {
        Some(ProfileFormat::WpaSupplicant)
    } else if vpn::looks_like_l2tp(text) {
        Some(ProfileFormat::L2tp)
    } else if vpn::looks_like_strongswan(text) {
        Some(ProfileFormat::StrongSwan)
    } else if openvpn::looks_like_openvpn(text) {
        Some(ProfileFormat::OpenVpn)
    } else if vpn::looks_like_pptp(text) {
        Some(ProfileFormat::Pptp)
    } else if vpn::looks_like_openconnect(text) {
        Some(ProfileFormat::OpenConnect)
    } else {
        None
    }
}

/// Converts a profile text into one or more profiles, detecting the format unless the options give it.
pub fn import_profiles(
    name_hint: &str,
    text: &str,
    options: &ImportOptions,
) -> Result<Vec<ImportedProfile>, ImportError> {
    let format = options.format.or_else(|| detect_format(text)).ok_or(ImportError::Unrecognised)?;
    match format {
        ProfileFormat::WireGuard => Ok(vec![import_wireguard(name_hint, text)?]),
        ProfileFormat::Keyfile => Ok(vec![import_keyfile_with(text, options)?]),
        ProfileFormat::WpaSupplicant => import_wpa_supplicant(text),
        ProfileFormat::OpenVpn => Ok(vec![import_openvpn(name_hint, text, options)?]),
        ProfileFormat::OpenConnect => Ok(vec![import_openconnect(name_hint, text)?]),
        ProfileFormat::StrongSwan => import_strongswan(name_hint, text),
        ProfileFormat::L2tp => import_l2tp(name_hint, text),
        ProfileFormat::Pptp => Ok(vec![import_pptp(name_hint, text)?]),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(profile: &ImportedProfile, group: &str, key: &str) -> String {
        String::try_from(profile.settings[group][key].clone()).unwrap()
    }

    fn list(profile: &ImportedProfile, group: &str, key: &str) -> Vec<String> {
        Vec::<String>::try_from(profile.settings[group][key].clone()).unwrap()
    }

    fn map(profile: &ImportedProfile, group: &str, key: &str) -> HashMap<String, String> {
        HashMap::<String, String>::try_from(profile.settings[group][key].clone()).unwrap()
    }

    const WIFI: &str = "[connection]
id=Office
uuid=11111111-2222-3333-4444-555555555555
type=wifi
autoconnect-priority=5
permissions=user:alice:;

[wifi]
ssid=Office
mode=infrastructure
hidden=true
mac-address=AA:BB:CC:DD:EE:FF
cloned-mac-address=random

[wifi-security]
key-mgmt=wpa-eap
proto=rsn;

[802-1x]
eap=tls;
identity=alice
ca-cert=/etc/ssl/ca.pem
client-cert=/home/a/client.pem
private-key=/home/a/key.pem
private-key-password=pw
private-key-password-flags=4
altsubject-matches=DNS:radius.example.com;DNS:radius2.example.com;
domain-suffix-match=example.com

[ipv4]
method=manual
address1=192.168.1.5/24,192.168.1.1
dns=1.1.1.1;
route1=10.0.0.0/8,192.168.1.254,100
route1_options=onlink=true

[ipv6]
method=ignore
";

    #[test]
    fn keyfiles_become_typed_settings() {
        let profile = import_keyfile(WIFI).unwrap();
        assert_eq!(profile.name, "Office");
        assert_eq!(text(&profile, "connection", "type"), "802-11-wireless");
        assert_eq!(i32::try_from(profile.settings["connection"]["autoconnect-priority"].clone()).unwrap(), 5);
        let wireless = &profile.settings["802-11-wireless"];
        assert_eq!(Vec::<u8>::try_from(wireless["ssid"].clone()).unwrap(), b"Office");
        assert_eq!(Vec::<u8>::try_from(wireless["mac-address"].clone()).unwrap(), [0xAA, 0xBB, 0xCC, 0xDD, 0xEE, 0xFF]);
        assert!(bool::try_from(wireless["hidden"].clone()).unwrap());
        assert_eq!(text(&profile, "802-11-wireless", "assigned-mac-address"), "random");
        assert!(!wireless.contains_key("cloned-mac-address"));
        assert_eq!(list(&profile, "802-11-wireless-security", "proto"), ["rsn"]);
        let x = &profile.settings["802-1x"];
        assert_eq!(list(&profile, "802-1x", "eap"), ["tls"]);
        assert_eq!(Vec::<u8>::try_from(x["ca-cert"].clone()).unwrap(), b"file:///etc/ssl/ca.pem\0");
        assert_eq!(u32::try_from(x["private-key-password-flags"].clone()).unwrap(), 4);
        assert_eq!(
            list(&profile, "802-1x", "altsubject-matches"),
            ["DNS:radius.example.com", "DNS:radius2.example.com"]
        );
        assert!(profile.guessed.is_empty());
    }

    #[test]
    fn addresses_dns_and_routes_are_translated() {
        let profile = import_keyfile(WIFI).unwrap();
        let ipv4 = &profile.settings["ipv4"];
        assert_eq!(text(&profile, "ipv4", "gateway"), "192.168.1.1");
        assert_eq!(Vec::<u32>::try_from(ipv4["dns"].clone()).unwrap(), [u32::from_ne_bytes([1, 1, 1, 1])]);
        assert!(ipv4.contains_key("address-data"));
        assert!(ipv4.contains_key("route-data"));
        assert_eq!(profile.ignored, ["ipv4.route1_options"]);
    }

    #[test]
    fn unknown_keys_are_guessed_and_overridable() {
        let source = "[connection]
id=Odd
type=ethernet

[ethernet]
future-flag=true
future-count=7
future-name=abc
";
        let profile = import_keyfile(source).unwrap();
        assert_eq!(profile.guessed.len(), 3);
        let ethernet = &profile.settings["802-3-ethernet"];
        assert!(bool::try_from(ethernet["future-flag"].clone()).unwrap());
        assert_eq!(u32::try_from(ethernet["future-count"].clone()).unwrap(), 7);
        assert_eq!(String::try_from(ethernet["future-name"].clone()).unwrap(), "abc");

        let mut options = ImportOptions::default();
        options.types.insert("802-3-ethernet.future-count".into(), ValueType::I64);
        options.types.insert("802-3-ethernet.future-name".into(), ValueType::Str);
        let profile = import_keyfile_with(source, &options).unwrap();
        assert_eq!(profile.guessed, ["802-3-ethernet.future-flag"]);
        assert_eq!(i64::try_from(profile.settings["802-3-ethernet"]["future-count"].clone()).unwrap(), 7);
    }

    #[test]
    fn bond_and_user_groups_are_flattened() {
        let source = "[connection]
id=Team
type=bond

[bond]
interface-name=bond0
mode=active-backup
miimon=100

[user]
org.example.note=hello
";
        let profile = import_keyfile(source).unwrap();
        let options = map(&profile, "bond", "options");
        assert_eq!(options["mode"], "active-backup");
        assert_eq!(options["miimon"], "100");
        assert!(!options.contains_key("interface-name"));
        assert_eq!(text(&profile, "bond", "interface-name"), "bond0");
        assert_eq!(map(&profile, "user", "data")["org.example.note"], "hello");
    }

    #[test]
    fn vpn_keyfiles_keep_data_and_secrets() {
        let source = "[connection]
id=Work
type=vpn

[vpn]
service-type=openvpn
user-name=bob
remote=vpn.example.com:1194
password-flags=1

[vpn-secrets]
password=hunter2
";
        let profile = import_keyfile(source).unwrap();
        assert_eq!(text(&profile, "vpn", "service-type"), "org.freedesktop.NetworkManager.openvpn");
        let data = map(&profile, "vpn", "data");
        assert_eq!(data["remote"], "vpn.example.com:1194");
        assert_eq!(data["password-flags"], "1");
        assert_eq!(map(&profile, "vpn", "secrets")["password"], "hunter2");
    }

    #[test]
    fn wireguard_configurations_become_profiles() {
        let source = "[Interface]
PrivateKey = cHJpdmF0ZQ==
Address = 10.0.0.2/32, fd00::2/128
ListenPort = 51820
DNS = 10.0.0.1, corp.example.com
PostUp = iptables -A FORWARD -i %i -j ACCEPT

[Peer]
PublicKey = cHVibGlj
Endpoint = vpn.example.com:51820
AllowedIPs = 0.0.0.0/0, ::/0
PersistentKeepalive = 25
";
        let profile = import_profiles("wg office!", source, &ImportOptions::default()).unwrap().remove(0);
        assert_eq!(text(&profile, "connection", "type"), "wireguard");
        assert_eq!(text(&profile, "connection", "interface-name"), "wgoffice");
        assert_eq!(u32::try_from(profile.settings["wireguard"]["listen-port"].clone()).unwrap(), 51820);
        assert!(profile.settings["wireguard"].contains_key("peers"));
        assert_eq!(text(&profile, "ipv4", "method"), "manual");
        assert_eq!(text(&profile, "ipv6", "method"), "manual");
        assert_eq!(list(&profile, "ipv4", "dns-search"), ["corp.example.com"]);
        assert_eq!(profile.ignored, ["Interface.postup"]);
    }

    #[test]
    fn wireguard_keyfile_peers_are_collected() {
        let source = "[connection]
id=wg0
type=wireguard
interface-name=wg0

[wireguard]
private-key=cHJpdmF0ZQ==

[wireguard-peer.cHVibGlj]
endpoint=1.2.3.4:51820
allowed-ips=10.0.0.0/24;
persistent-keepalive=20
";
        let profile = import_keyfile(source).unwrap();
        assert!(profile.settings["wireguard"].contains_key("peers"));
    }

    const SUPPLICANT: &str = "ctrl_interface=/run/wpa_supplicant

network={
    ssid=\"Home\"
    psk=\"correct horse\"
    key_mgmt=WPA-PSK SAE
    proto=RSN
    pairwise=CCMP
    priority=3
    scan_ssid=1
    ieee80211w=2
    id_str=\"home\"
}

network={
    ssid=\"Campus\"
    key_mgmt=WPA-EAP
    eap=PEAP
    identity=\"alice\"
    password=\"secret\"
    ca_cert=\"/etc/ca.pem\"
    phase1=\"peapver=0\"
    phase2=\"auth=MSCHAPV2\"
    altsubject_match=\"DNS:a.example;DNS:b.example\"
}

network={
    ssid=4f6c64
    key_mgmt=NONE
    wep_key0=\"abcde\"
    wep_tx_keyidx=0
    auth_alg=SHARED
}
";

    #[test]
    fn wpa_supplicant_blocks_become_profiles() {
        let profiles = import_wpa_supplicant(SUPPLICANT).unwrap();
        assert_eq!(profiles.len(), 3);

        let home = &profiles[0];
        assert_eq!(home.name, "Home");
        assert_eq!(text(home, "802-11-wireless-security", "key-mgmt"), "wpa-psk");
        assert_eq!(text(home, "802-11-wireless-security", "psk"), "correct horse");
        assert_eq!(list(home, "802-11-wireless-security", "proto"), ["rsn"]);
        assert_eq!(list(home, "802-11-wireless-security", "pairwise"), ["ccmp"]);
        assert_eq!(i32::try_from(home.settings["802-11-wireless-security"]["pmf"].clone()).unwrap(), 3);
        assert_eq!(i32::try_from(home.settings["connection"]["autoconnect-priority"].clone()).unwrap(), 3);
        assert!(bool::try_from(home.settings["802-11-wireless"]["hidden"].clone()).unwrap());
        assert_eq!(home.ignored, ["id_str"]);

        let campus = &profiles[1];
        assert_eq!(text(campus, "802-11-wireless-security", "key-mgmt"), "wpa-eap");
        assert_eq!(list(campus, "802-1x", "eap"), ["peap"]);
        assert_eq!(text(campus, "802-1x", "phase2-auth"), "mschapv2");
        assert_eq!(text(campus, "802-1x", "phase1-peapver"), "0");
        assert_eq!(list(campus, "802-1x", "altsubject-matches"), ["DNS:a.example", "DNS:b.example"]);
        assert_eq!(Vec::<u8>::try_from(campus.settings["802-1x"]["ca-cert"].clone()).unwrap(), b"file:///etc/ca.pem\0");

        let old = &profiles[2];
        assert_eq!(old.name, "Old");
        assert_eq!(text(old, "802-11-wireless-security", "key-mgmt"), "none");
        assert_eq!(text(old, "802-11-wireless-security", "wep-key0"), "abcde");
        assert_eq!(text(old, "802-11-wireless-security", "auth-alg"), "shared");
    }

    #[test]
    fn wpa_supplicant_rejects_bad_blocks() {
        let bad_psk = "network={\n ssid=\"x\"\n psk=\"short\"\n}\n";
        assert!(matches!(import_wpa_supplicant(bad_psk), Err(ImportError::Invalid { .. })));
        assert!(matches!(import_wpa_supplicant("network={\n key_mgmt=NONE\n}\n"), Err(ImportError::Missing(_))));
    }

    const OVPN: &str = "client
dev tun
proto tcp
remote vpn.example.com 443
remote backup.example.com 1194 udp
nobind
persist-key
ca ca.crt
cert client.crt
key client.key
tls-auth ta.key 1
cipher AES-256-GCM
auth SHA256
remote-cert-tls server
comp-lzo adaptive
auth-user-pass
verify-x509-name vpn.example.com name
route 10.0.0.0 255.0.0.0
";

    #[test]
    fn openvpn_configurations_become_vpn_profiles() {
        let options = ImportOptions { base_dir: Some("/etc/vpn".into()), ..Default::default() };
        let profile = import_openvpn("Work", OVPN, &options).unwrap();
        assert_eq!(text(&profile, "connection", "type"), "vpn");
        assert_eq!(text(&profile, "vpn", "service-type"), "org.freedesktop.NetworkManager.openvpn");
        let data = map(&profile, "vpn", "data");
        assert_eq!(data["remote"], "vpn.example.com:443:tcp, backup.example.com:1194:udp");
        assert_eq!(data["proto-tcp"], "yes");
        assert_eq!(data["dev-type"], "tun");
        assert_eq!(data["ca"], "/etc/vpn/ca.crt");
        assert_eq!(data["ta"], "/etc/vpn/ta.key");
        assert_eq!(data["ta-dir"], "1");
        assert_eq!(data["connection-type"], "password-tls");
        assert_eq!(data["password-flags"], "1");
        assert_eq!(data["comp-lzo"], "adaptive");
        assert_eq!(data["verify-x509-name"], "vpn.example.com:name");
        assert!(profile.ignored.is_empty());
        assert!(profile.settings["ipv4"].contains_key("route-data"));
    }

    #[test]
    fn openvpn_inline_blocks_are_written_privately() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let source = "client\nremote vpn.example.com\n<ca>\nCA-DATA\n</ca>\n<tls-crypt>\nKEY-DATA\n</tls-crypt>\n";
        let options = ImportOptions { output_dir: Some(dir.path().to_path_buf()), ..Default::default() };
        let profile = import_openvpn("Home vpn", source, &options).unwrap();
        let data = map(&profile, "vpn", "data");
        assert_eq!(data["connection-type"], "tls");
        assert_eq!(data["remote"], "vpn.example.com:1194:udp");
        let ca = std::path::PathBuf::from(&data["ca"]);
        assert_eq!(std::fs::read_to_string(&ca).unwrap(), "CA-DATA\n");
        assert_eq!(std::fs::metadata(&ca).unwrap().permissions().mode() & 0o777, 0o600);
        assert!(data["tls-crypt"].ends_with("Home_vpn-tls-crypt.pem"));
    }

    #[test]
    fn detection_picks_the_right_importer() {
        let options = ImportOptions::default();
        assert_eq!(import_profiles("x", SUPPLICANT, &options).unwrap().len(), 3);
        assert_eq!(import_profiles("x", WIFI, &options).unwrap()[0].name, "Office");
        assert_eq!(import_profiles("Work", OVPN, &options).unwrap()[0].name, "Work");
        assert!(matches!(import_profiles("x", "hello", &options), Err(ImportError::Unrecognised)));
    }

    #[test]
    fn bad_keyfiles_are_rejected() {
        assert!(matches!(import_keyfile("[connection]\ntype=wifi\n"), Err(ImportError::Missing(_))));
        assert!(matches!(
            import_keyfile("[connection]\nid=a\ntype=wifi\nautoconnect=maybe\n"),
            Err(ImportError::Invalid { .. })
        ));
        assert!(matches!(
            super::import_wireguard("w", "[Interface]\nAddress=10.0.0.2/32\n"),
            Err(ImportError::Missing(_))
        ));
    }

    #[test]
    fn openvpn_routes_dns_and_pull_filters_go_to_ipv4() {
        let source = "client\nremote h.example.com\nroute 10.1.0.0 255.255.0.0 10.8.0.1 50\nroute 192.168.5.7\ndhcp-option DNS 10.8.0.1\ndhcp-option DOMAIN corp.example\ndhcp-option PROXY x\nroute-nopull\n";
        let profile = import_openvpn("r", source, &ImportOptions::default()).unwrap();
        let ipv4 = &profile.settings["ipv4"];
        assert!(ipv4.contains_key("route-data"));
        assert_eq!(Vec::<u32>::try_from(ipv4["dns"].clone()).unwrap(), [u32::from_ne_bytes([10, 8, 0, 1])]);
        assert_eq!(list(&profile, "ipv4", "dns-search"), ["corp.example"]);
        assert!(bool::try_from(ipv4["never-default"].clone()).unwrap());
        assert_eq!(profile.ignored, ["dhcp-option.PROXY"]);
        let redirected =
            import_openvpn("r", "client\nremote h\nredirect-gateway def1\nroute-nopull\n", &ImportOptions::default())
                .unwrap();
        assert!(!redirected.settings["ipv4"].contains_key("never-default"));
    }

    #[test]
    fn openconnect_configurations_become_profiles() {
        let source = "# corporate
server=vpn.example.com
protocol=gp
user=alice
cafile=/etc/ca.pem
servercert=pin-sha256:abc
no-dtls
csd-wrapper=/usr/libexec/hip
authgroup=staff
";
        let profile = import_profiles("Corp", source, &ImportOptions::default()).unwrap().remove(0);
        assert_eq!(text(&profile, "vpn", "service-type"), "org.freedesktop.NetworkManager.openconnect");
        assert_eq!(text(&profile, "vpn", "user-name"), "alice");
        let data = map(&profile, "vpn", "data");
        assert_eq!(data["gateway"], "vpn.example.com");
        assert_eq!(data["protocol"], "gp");
        assert_eq!(data["gwcert"], "pin-sha256:abc");
        assert_eq!(data["disable_udp"], "yes");
        assert_eq!(data["enable_csd_trojan"], "yes");
        assert_eq!(data["authtype"], "password");
        assert_eq!(profile.ignored, ["authgroup"]);
        let cert_only = import_openconnect("c", "server=h\ncertificate=/c.pem\nsslkey=/k.pem\n").unwrap();
        assert_eq!(map(&cert_only, "vpn", "data")["authtype"], "cert");
    }

    #[test]
    fn strongswan_connections_become_profiles() {
        let source = "config setup
    charondebug=\"ike 1\"

conn %default
    keyexchange=ikev2

conn office
    right=gw.example.com
    rightid=@gw.example.com
    rightca=\"/etc/ipsec.d/cacerts/ca.pem\"
    leftauth=eap-mschapv2
    eap_identity=alice
    leftsourceip=%config
    ike=aes256-sha256-modp2048!
    esp=aes256-sha256
    forceencaps=yes
    dpdaction=restart

conn site
    right=203.0.113.9
    authby=secret
";
        let profiles = import_profiles("x", source, &ImportOptions::default()).unwrap();
        assert_eq!(profiles.len(), 2);
        let office = &profiles[0];
        assert_eq!(office.name, "office");
        assert_eq!(text(office, "vpn", "service-type"), "org.freedesktop.NetworkManager.strongswan");
        assert_eq!(text(office, "vpn", "user-name"), "alice");
        let data = map(office, "vpn", "data");
        assert_eq!((data["method"].as_str(), data["virtual"].as_str(), data["encap"].as_str()), ("eap", "yes", "yes"));
        assert_eq!(data["ike"], "aes256-sha256-modp2048");
        assert_eq!(data["proposal"], "yes");
        assert_eq!(data["remote-identity"], "@gw.example.com");
        assert_eq!(office.ignored, ["dpdaction"]);
        assert_eq!(map(&profiles[1], "vpn", "data")["method"], "psk");
    }

    #[test]
    fn l2tp_and_pptp_configurations_become_profiles() {
        let l2tp = "[lac work]
lns = l2tp.example.com
name = bob
refuse pap = yes
require chap = yes
redial = yes

mtu 1400
lcp-echo-interval 30
";
        let profile = import_profiles("x", l2tp, &ImportOptions::default()).unwrap().remove(0);
        assert_eq!(profile.name, "work");
        assert_eq!(text(&profile, "vpn", "user-name"), "bob");
        let data = map(&profile, "vpn", "data");
        assert_eq!(data["gateway"], "l2tp.example.com");
        assert_eq!(data["refuse-pap"], "yes");
        assert_eq!(data["mtu"], "1400");
        assert_eq!(data["lcp-echo-interval"], "30");
        assert_eq!(profile.ignored, ["redial"]);

        let pptp = "pty \"pptp pptp.example.com --nolaunchpppd\"
name carol
remotename pptp
require-mppe-128
refuse-eap
refuse-mschap-v2
nobsdcomp
lcp-echo-failure 5
persist
";
        let profile = import_profiles("Branch", pptp, &ImportOptions::default()).unwrap().remove(0);
        assert_eq!(profile.name, "Branch");
        assert_eq!(text(&profile, "vpn", "service-type"), "org.freedesktop.NetworkManager.pptp");
        let data = map(&profile, "vpn", "data");
        assert_eq!(data["gateway"], "pptp.example.com");
        assert_eq!(data["require-mppe-128"], "yes");
        assert_eq!(data["refuse-mschapv2"], "yes");
        assert_eq!(data["lcp-echo-failure"], "5");
        assert_eq!(profile.ignored, ["remotename", "persist"]);
    }

    #[test]
    fn ethtool_and_sriov_keyfile_groups_are_typed() {
        let source = "[connection]
id=Nic
type=ethernet

[ethtool]
feature-rx=true
feature-tso=ignore
coalesce-rx-usecs=50
ring-rx=256
pause-autoneg=false

[sriov]
total-vfs=4
autoprobe-drivers=true
vfs=0 mac=00:11:22:33:44:55
";
        let profile = import_keyfile(source).unwrap();
        let ethtool = &profile.settings["ethtool"];
        assert_eq!(i32::try_from(ethtool["feature-rx"].clone()).unwrap(), 1);
        assert_eq!(i32::try_from(ethtool["feature-tso"].clone()).unwrap(), -1);
        assert_eq!(i32::try_from(ethtool["pause-autoneg"].clone()).unwrap(), 0);
        assert_eq!(u32::try_from(ethtool["coalesce-rx-usecs"].clone()).unwrap(), 50);
        assert_eq!(u32::try_from(ethtool["ring-rx"].clone()).unwrap(), 256);
        assert_eq!(u32::try_from(profile.settings["sriov"]["total-vfs"].clone()).unwrap(), 4);
        assert_eq!(profile.ignored, ["sriov.vfs"]);
        assert!(profile.guessed.is_empty());
    }

    #[test]
    fn a_format_can_be_forced() {
        let options = ImportOptions { format: Some(ProfileFormat::OpenConnect), ..Default::default() };
        let profile = import_profiles("x", "server=h\n", &options).unwrap().remove(0);
        assert_eq!(map(&profile, "vpn", "data")["gateway"], "h");
        assert_eq!(detect_format("[lac a]\nlns=h\n"), Some(ProfileFormat::L2tp));
        assert_eq!(detect_format("nothing here"), None);
    }
}
