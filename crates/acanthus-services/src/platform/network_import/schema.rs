use zbus::{OwnedVariant as OwnedValue, Variant as ZValue};

use super::{ImportError, cert_value, invalid, owned, parse_bool, parse_hex, parse_mac, split_list};

/// The type of a profile value, deciding how its text is stored.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ValueType {
    /// A string.
    Str,
    /// An unsigned 32-bit integer.
    U32,
    /// A signed 32-bit integer.
    I32,
    /// A signed 64-bit integer.
    I64,
    /// An unsigned 64-bit integer.
    U64,
    /// A boolean.
    Bool,
    /// A list of strings.
    StrList,
    /// A Wi-Fi network name as bytes.
    Ssid,
    /// A hardware address as bytes.
    Mac,
    /// A certificate or key, as a path or inline data.
    Cert,
    /// Bytes written in hexadecimal.
    Hex,
    /// A value that is on, off or default.
    Ternary,
}

pub(crate) fn dbus_key<'a>(group: &str, key: &'a str) -> &'a str {
    match (group, key) {
        ("802-3-ethernet" | "802-11-wireless", "cloned-mac-address") => "assigned-mac-address",
        _ => key,
    }
}

pub(crate) fn flattened_property(group: &str) -> Option<&'static str> {
    match group {
        "bond" => Some("options"),
        "user" => Some("data"),
        _ => None,
    }
}

fn one_of(key: &str, keys: &[&str]) -> bool {
    keys.contains(&key)
}

pub(crate) fn value_type(group: &str, key: &str) -> Option<ValueType> {
    use ValueType::*;
    if key.ends_with("-flags") && key != "ethtool" {
        return Some(U32);
    }
    match group {
        "connection" => match key {
            "autoconnect" | "read-only" => Some(Bool),
            "autoconnect-priority"
            | "autoconnect-retries"
            | "multi-connect"
            | "autoconnect-slaves"
            | "metered"
            | "lldp"
            | "auth-retries"
            | "mdns"
            | "llmnr"
            | "dns-over-tls"
            | "wait-device-timeout"
            | "wait-activation-delay" => Some(I32),
            "timestamp" => Some(U64),
            "gateway-ping-timeout" => Some(U32),
            "permissions" | "secondaries" => Some(StrList),
            "id" | "uuid" | "stable-id" | "type" | "interface-name" | "zone" | "master" | "slave-type" | "mud-url" => {
                Some(Str)
            }
            _ => None,
        },
        "802-3-ethernet" => match key {
            "port"
            | "duplex"
            | "wake-on-lan-password"
            | "assigned-mac-address"
            | "generate-mac-address-mask"
            | "cloned-mac-address" => Some(Str),
            "speed" | "mtu" | "wake-on-lan" | "s390-nettype" => Some(U32),
            "auto-negotiate" => Some(Bool),
            "mac-address" => Some(Mac),
            "mac-address-blacklist" | "s390-subchannels" => Some(StrList),
            "accept-all-mac-addresses" => Some(I32),
            _ => None,
        },
        "802-11-wireless" => match key {
            "ssid" => Some(Ssid),
            "mode" | "band" | "assigned-mac-address" | "cloned-mac-address" | "generate-mac-address-mask" => Some(Str),
            "channel" | "rate" | "tx-power" | "mtu" | "powersave" | "wake-on-wlan" | "mac-address-randomization" => {
                Some(U32)
            }
            "bssid" | "mac-address" => Some(Mac),
            "mac-address-blacklist" | "seen-bssids" => Some(StrList),
            "hidden" => Some(Bool),
            "ap-isolation" => Some(I32),
            _ => None,
        },
        "802-11-wireless-security" => match key {
            "key-mgmt" | "auth-alg" | "leap-username" | "wep-key0" | "wep-key1" | "wep-key2" | "wep-key3" | "psk"
            | "leap-password" => Some(Str),
            "wep-tx-keyidx" | "wep-key-type" | "wps-method" => Some(U32),
            "proto" | "pairwise" | "group" => Some(StrList),
            "pmf" | "fils" => Some(I32),
            _ => None,
        },
        "802-1x" => match key {
            "eap" | "altsubject-matches" | "phase2-altsubject-matches" => Some(StrList),
            "ca-cert" | "client-cert" | "private-key" | "phase2-ca-cert" | "phase2-client-cert"
            | "phase2-private-key" => Some(Cert),
            "password-raw" => Some(Hex),
            "system-ca-certs" | "optional" => Some(Bool),
            "auth-timeout" => Some(I32),
            "identity"
            | "anonymous-identity"
            | "pac-file"
            | "ca-cert-password"
            | "ca-path"
            | "subject-match"
            | "domain-suffix-match"
            | "domain-match"
            | "client-cert-password"
            | "phase1-peapver"
            | "phase1-peaplabel"
            | "phase1-fast-provisioning"
            | "phase2-auth"
            | "phase2-autheap"
            | "phase2-ca-path"
            | "phase2-subject-match"
            | "phase2-domain-suffix-match"
            | "phase2-domain-match"
            | "phase2-client-cert-password"
            | "password"
            | "private-key-password"
            | "phase2-private-key-password"
            | "pin" => Some(Str),
            _ => None,
        },
        "ipv4" | "ipv6" => match key {
            "method"
            | "gateway"
            | "dhcp-client-id"
            | "dhcp-iaid"
            | "dhcp-hostname"
            | "dhcp-fqdn"
            | "dhcp-vendor-class-identifier"
            | "token"
            | "dhcp-duid" => Some(Str),
            "ignore-auto-routes" | "ignore-auto-dns" | "never-default" | "may-fail" | "dhcp-send-hostname" => {
                Some(Bool)
            }
            "route-metric" => Some(I64),
            "route-table" | "dhcp-hostname-flags" | "link-local" => Some(U32),
            "dns-priority" | "dhcp-timeout" | "dad-timeout" | "required-timeout" | "auto-route-ext-gw"
            | "replace-local-rule" | "dhcp-send-release" | "addr-gen-mode" | "ip6-privacy" | "ra-timeout" => Some(I32),
            "dns-search" | "dns-options" | "dhcp-reject-servers" => Some(StrList),
            _ => None,
        },
        "wireguard" => match key {
            "private-key" | "ip4-auto-default-route" => Some(Str),
            "listen-port" | "fwmark" | "mtu" | "ip4-auto-default-route-flags" => Some(U32),
            "peer-routes" => Some(Bool),
            _ => None,
        },
        "bridge" => match key {
            "mac-address" => Some(Mac),
            "stp"
            | "multicast-snooping"
            | "vlan-filtering"
            | "vlan-stats-enabled"
            | "multicast-query-use-ifaddr"
            | "multicast-querier" => Some(Bool),
            "priority"
            | "forward-delay"
            | "hello-time"
            | "max-age"
            | "ageing-time"
            | "group-forward-mask"
            | "multicast-hash-max"
            | "multicast-last-member-count"
            | "multicast-startup-query-count"
            | "vlan-default-pvid" => Some(U32),
            "group-address" | "multicast-router" | "vlan-protocol" => Some(Str),
            _ => None,
        },
        "bridge-port" => match key {
            "priority" | "path-cost" => Some(U32),
            "hairpin-mode" => Some(Bool),
            _ => None,
        },
        "team" => (key == "config").then_some(Str),
        "team-port" => (key == "config").then_some(Str),
        "vlan" => match key {
            "id" | "flags" => Some(U32),
            "parent" => Some(Str),
            "ingress-priority-map" | "egress-priority-map" => Some(StrList),
            _ => None,
        },
        "proxy" => match key {
            "method" => Some(I32),
            "browser-only" => Some(Bool),
            "pac-url" | "pac-script" => Some(Str),
            _ => None,
        },
        "gsm" => match key {
            "number"
            | "username"
            | "password"
            | "apn"
            | "network-id"
            | "pin"
            | "device-id"
            | "sim-id"
            | "sim-operator-id"
            | "mobile-country-code"
            | "mobile-network-code" => Some(Str),
            "home-only" | "auto-config" => Some(Bool),
            "mtu" => Some(U32),
            _ => None,
        },
        "cdma" => match key {
            "number" | "username" | "password" => Some(Str),
            "mtu" => Some(U32),
            _ => None,
        },
        "pppoe" => match key {
            "service" | "username" | "password" | "parent" => Some(Str),
            _ => None,
        },
        "infiniband" => match key {
            "mac-address" => Some(Hex),
            "mtu" => Some(U32),
            "p-key" => Some(I32),
            "transport-mode" | "parent" => Some(Str),
            _ => None,
        },
        "tun" => match key {
            "mode" => Some(U32),
            "owner" | "group" => Some(Str),
            "pi" | "vnet-hdr" | "multi-queue" => Some(Bool),
            _ => None,
        },
        "match" => one_of(key, &["interface-name", "kernel-command-line", "driver", "path"]).then_some(StrList),
        "ovs-bridge" | "ovs-port" | "ovs-interface" => match key {
            "type" | "fail-mode" | "tag" if key != "tag" => Some(Str),
            "tag" | "mcast-snooping-enable" | "rstp-enable" | "stp-enable" | "lacp" => Some(U32),
            _ => None,
        },
        "ethtool" => {
            if key.starts_with("feature-") || key.starts_with("pause-") {
                Some(Ternary)
            } else if key.starts_with("coalesce-") || key.starts_with("ring-") || key == "fec-mode" {
                Some(U32)
            } else {
                None
            }
        }
        "sriov" => match key {
            "total-vfs" => Some(U32),
            "autoprobe-drivers" => Some(Ternary),
            _ => None,
        },
        "bond" => (key == "interface-name").then_some(Str),
        _ => None,
    }
}

pub(crate) fn convert(kind: ValueType, key: &str, value: &str) -> Result<OwnedValue, ImportError> {
    let number = |v: &str| v.parse::<i64>().map_err(|_| invalid(key, value));
    Ok(match kind {
        ValueType::Str => owned(ZValue::from(value)),
        ValueType::U32 => owned(ZValue::from(u32::try_from(number(value)?).map_err(|_| invalid(key, value))?)),
        ValueType::I32 => owned(ZValue::from(i32::try_from(number(value)?).map_err(|_| invalid(key, value))?)),
        ValueType::I64 => owned(ZValue::from(number(value)?)),
        ValueType::U64 => owned(ZValue::from(u64::try_from(number(value)?).map_err(|_| invalid(key, value))?)),
        ValueType::Bool => owned(ZValue::from(parse_bool(key, value)?)),
        ValueType::StrList => owned(ZValue::from(split_list(value, ';'))),
        ValueType::Ssid => owned(ZValue::from(value.as_bytes().to_vec())),
        ValueType::Mac => owned(ZValue::from(parse_mac(key, value)?)),
        ValueType::Cert => owned(ZValue::from(cert_value(value))),
        ValueType::Hex => owned(ZValue::from(parse_hex(key, value)?)),
        ValueType::Ternary => owned(ZValue::from(match value.to_ascii_lowercase().as_str() {
            "true" | "yes" | "on" | "1" => 1i32,
            "false" | "no" | "off" | "0" => 0,
            "ignore" | "-1" => -1,
            _ => return Err(invalid(key, value)),
        })),
    })
}

pub(crate) fn guess(value: &str) -> ValueType {
    let lower = value.to_ascii_lowercase();
    if lower == "true" || lower == "false" {
        ValueType::Bool
    } else if !value.is_empty() && value.chars().all(|c| c.is_ascii_digit()) {
        ValueType::U32
    } else if value.strip_prefix('-').is_some_and(|d| !d.is_empty() && d.chars().all(|c| c.is_ascii_digit())) {
        ValueType::I32
    } else {
        ValueType::Str
    }
}
