use std::collections::HashMap;

use zbus::{OwnedVariant as OwnedValue, Variant as ZValue};

use super::schema::{convert, dbus_key, flattened_property, guess, value_type};
use super::{
    ImportError, ImportOptions, ImportedProfile, address_entry, dict, dict_array, dns_value, invalid, owned,
    parse_bool, parse_groups, route_entry, string_map,
};

fn canonical_group(name: &str) -> &str {
    match name {
        "wifi" => "802-11-wireless",
        "wifi-security" => "802-11-wireless-security",
        "ethernet" => "802-3-ethernet",
        other => other,
    }
}

fn canonical_type(value: &str) -> &str {
    match value {
        "wifi" => "802-11-wireless",
        "ethernet" => "802-3-ethernet",
        other => other,
    }
}

fn peer_entry(public_key: &str, entries: &[(String, String)]) -> Result<ZValue<'static>, ImportError> {
    let mut fields: Vec<(&str, ZValue<'static>)> = vec![("public-key", ZValue::from(public_key.to_owned()))];
    for (key, value) in entries {
        match key.as_str() {
            "endpoint" => fields.push(("endpoint", ZValue::from(value.clone()))),
            "preshared-key" => fields.push(("preshared-key", ZValue::from(value.clone()))),
            "allowed-ips" => fields.push(("allowed-ips", ZValue::from(super::split_list(value, ';')))),
            "persistent-keepalive" => {
                let n = value.parse::<u32>().map_err(|_| invalid(key, value))?;
                fields.push(("persistent-keepalive", ZValue::from(n)));
            }
            "preshared-key-flags" => {
                let n = value.parse::<u32>().map_err(|_| invalid(key, value))?;
                fields.push(("preshared-key-flags", ZValue::from(n)));
            }
            _ => {}
        }
    }
    Ok(dict(fields))
}

fn numbered(key: &str, prefix: &str) -> bool {
    key.strip_prefix(prefix).is_some_and(|n| !n.is_empty() && n.chars().all(|c| c.is_ascii_digit()))
}

/// Imports a NetworkManager keyfile.
pub fn import_keyfile(text: &str) -> Result<ImportedProfile, ImportError> {
    import_keyfile_with(text, &ImportOptions::default())
}

/// Like [`import_keyfile`] with explicit options.
pub fn import_keyfile_with(text: &str, options: &ImportOptions) -> Result<ImportedProfile, ImportError> {
    let groups = parse_groups(text);
    let mut profile = ImportedProfile::default();
    let mut peers: Vec<ZValue<'static>> = Vec::new();
    for (raw_name, entries) in &groups {
        let name = canonical_group(raw_name);
        if let Some(public_key) = name.strip_prefix("wireguard-peer.") {
            peers.push(peer_entry(public_key, entries)?);
            continue;
        }
        let mut properties: HashMap<String, OwnedValue> = HashMap::new();
        if name == "vpn-secrets" {
            profile.settings.entry("vpn".into()).or_default().insert("secrets".into(), owned(string_map(entries)));
            continue;
        }
        if name == "vpn" {
            let mut data = Vec::new();
            for (key, value) in entries {
                match key.as_str() {
                    "service-type" => {
                        let service = if value.contains('.') {
                            value.clone()
                        } else {
                            format!("org.freedesktop.NetworkManager.{value}")
                        };
                        properties.insert(key.clone(), owned(ZValue::from(service)));
                    }
                    "user-name" => {
                        properties.insert(key.clone(), owned(ZValue::from(value.as_str())));
                    }
                    "persistent" => {
                        properties.insert(key.clone(), owned(ZValue::from(parse_bool(key, value)?)));
                    }
                    _ => data.push((key.clone(), value.clone())),
                }
            }
            properties.insert("data".into(), owned(string_map(&data)));
        } else if let Some(target) = flattened_property(name) {
            let (own, flat): (Vec<_>, Vec<_>) =
                entries.iter().cloned().partition(|(key, _)| value_type(name, key).is_some());
            for (key, value) in own {
                let kind = value_type(name, &key).expect("partitioned");
                properties.insert(key.clone(), convert(kind, &key, &value)?);
            }
            properties.insert(target.to_owned(), owned(string_map(&flat)));
        } else if name == "ipv4" || name == "ipv6" {
            let mut addresses = Vec::new();
            let mut routes = Vec::new();
            let mut gateway = None;
            for (key, value) in entries {
                if numbered(key, "address") {
                    let (ip, prefix, gw) = address_entry(name, value)?;
                    addresses.push(dict(vec![("address", ZValue::from(ip)), ("prefix", ZValue::from(prefix))]));
                    gateway = gateway.or(gw);
                } else if numbered(key, "route") {
                    routes.push(route_entry(name, value)?);
                } else if key.starts_with("route") && key.ends_with("_options") {
                    profile.ignored.push(format!("{raw_name}.{key}"));
                } else if key == "dns" {
                    properties.insert(key.clone(), dns_value(name, value)?);
                } else {
                    insert_typed(&mut properties, &mut profile.guessed, options, name, key, value)?;
                }
            }
            if !addresses.is_empty() {
                properties.insert("address-data".into(), owned(dict_array(addresses)));
            }
            if !routes.is_empty() {
                properties.insert("route-data".into(), owned(dict_array(routes)));
            }
            if let Some(gateway) = gateway
                && !properties.contains_key("gateway")
            {
                properties.insert("gateway".into(), owned(ZValue::from(gateway)));
            }
        } else {
            for (key, value) in entries {
                if name == "sriov" && (key == "vfs" || key.starts_with("vf.")) {
                    profile.ignored.push(format!("{raw_name}.{key}"));
                    continue;
                }
                let value = if name == "connection" && key == "type" { canonical_type(value) } else { value };
                insert_typed(&mut properties, &mut profile.guessed, options, name, key, value)?;
            }
        }
        profile.settings.entry(name.to_owned()).or_default().extend(properties);
    }
    if !peers.is_empty() {
        profile.settings.entry("wireguard".into()).or_default().insert("peers".into(), owned(dict_array(peers)));
    }
    let connection = profile.settings.get("connection").ok_or(ImportError::Unrecognised)?;
    profile.name = connection
        .get("id")
        .and_then(|v| String::try_from(v.clone()).ok())
        .ok_or(ImportError::Missing("connection id"))?;
    if !connection.contains_key("type") {
        return Err(ImportError::Missing("connection type"));
    }
    Ok(profile)
}

fn insert_typed(
    properties: &mut HashMap<String, OwnedValue>,
    guessed: &mut Vec<String>,
    options: &ImportOptions,
    group: &str,
    key: &str,
    value: &str,
) -> Result<(), ImportError> {
    let path = format!("{group}.{key}");
    let kind = match options.types.get(&path).copied().or_else(|| value_type(group, key)) {
        Some(kind) => kind,
        None => {
            guessed.push(path);
            guess(value)
        }
    };
    properties.insert(dbus_key(group, key).to_owned(), convert(kind, key, value)?);
    Ok(())
}
