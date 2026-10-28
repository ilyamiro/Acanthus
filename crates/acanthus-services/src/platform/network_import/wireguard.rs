use std::collections::HashMap;

use zbus::{OwnedVariant as OwnedValue, Variant as ZValue};

use super::{
    ImportError, ImportedProfile, address_entry, dict, dict_array, dns_value, invalid, owned, parse_groups, split_list,
    valid_ip,
};

pub fn import_wireguard(name: &str, text: &str) -> Result<ImportedProfile, ImportError> {
    let groups = parse_groups(text);
    let mut profile = ImportedProfile { name: name.to_owned(), ..Default::default() };
    let mut wireguard: HashMap<String, OwnedValue> = HashMap::new();
    let mut peers: Vec<ZValue<'static>> = Vec::new();
    let mut addresses: Vec<String> = Vec::new();
    let mut dns: Vec<String> = Vec::new();
    let mut search: Vec<String> = Vec::new();
    let mut seen_interface = false;
    for (group, entries) in &groups {
        match group.to_ascii_lowercase().as_str() {
            "interface" => {
                seen_interface = true;
                for (key, value) in entries {
                    match key.to_ascii_lowercase().as_str() {
                        "privatekey" => {
                            wireguard.insert("private-key".into(), owned(ZValue::from(value.as_str())));
                        }
                        lower @ ("listenport" | "fwmark" | "mtu") => {
                            let target = match lower {
                                "listenport" => "listen-port",
                                other => other,
                            };
                            let n = if target == "fwmark" && value.eq_ignore_ascii_case("off") {
                                0
                            } else {
                                value.parse::<u32>().map_err(|_| invalid(key, value))?
                            };
                            wireguard.insert(target.into(), owned(ZValue::from(n)));
                        }
                        "address" => addresses.extend(split_list(value, ',')),
                        "dns" => {
                            for item in split_list(value, ',') {
                                if valid_ip("ipv4", &item) || valid_ip("ipv6", &item) {
                                    dns.push(item);
                                } else {
                                    search.push(item);
                                }
                            }
                        }
                        other => profile.ignored.push(format!("Interface.{other}")),
                    }
                }
            }
            "peer" => {
                let mut fields: Vec<(&str, ZValue<'static>)> = Vec::new();
                for (key, value) in entries {
                    match key.to_ascii_lowercase().as_str() {
                        "publickey" => fields.push(("public-key", ZValue::from(value.clone()))),
                        "presharedkey" => fields.push(("preshared-key", ZValue::from(value.clone()))),
                        "endpoint" => fields.push(("endpoint", ZValue::from(value.clone()))),
                        "allowedips" => fields.push(("allowed-ips", ZValue::from(split_list(value, ',')))),
                        "persistentkeepalive" => {
                            let n = value.parse::<u32>().map_err(|_| invalid(key, value))?;
                            fields.push(("persistent-keepalive", ZValue::from(n)));
                        }
                        other => profile.ignored.push(format!("Peer.{other}")),
                    }
                }
                if !fields.iter().any(|(k, _)| *k == "public-key") {
                    return Err(ImportError::Missing("peer public key"));
                }
                peers.push(dict(fields));
            }
            _ => {}
        }
    }
    if !seen_interface {
        return Err(ImportError::Unrecognised);
    }
    if !wireguard.contains_key("private-key") {
        return Err(ImportError::Missing("private key"));
    }
    if !peers.is_empty() {
        wireguard.insert("peers".into(), owned(dict_array(peers)));
    }
    let interface: String =
        name.chars().filter(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '=')).take(15).collect();
    profile.settings.insert(
        "connection".into(),
        HashMap::from([
            ("id".to_owned(), owned(ZValue::from(name))),
            ("type".to_owned(), owned(ZValue::from("wireguard"))),
            ("interface-name".to_owned(), owned(ZValue::from(interface))),
        ]),
    );
    profile.settings.insert("wireguard".into(), wireguard);
    for family in ["ipv4", "ipv6"] {
        let own: Vec<&String> =
            addresses.iter().filter(|a| valid_ip(family, a.split('/').next().unwrap_or_default())).collect();
        let mut properties: HashMap<String, OwnedValue> = HashMap::new();
        if own.is_empty() {
            properties.insert("method".into(), owned(ZValue::from("disabled")));
        } else {
            properties.insert("method".into(), owned(ZValue::from("manual")));
            let mut entries = Vec::new();
            for address in own {
                let (ip, prefix, _) = address_entry(family, address)?;
                entries.push(dict(vec![("address", ZValue::from(ip)), ("prefix", ZValue::from(prefix))]));
            }
            properties.insert("address-data".into(), owned(dict_array(entries)));
            let servers: Vec<&str> = dns.iter().filter(|d| valid_ip(family, d)).map(String::as_str).collect();
            if !servers.is_empty() {
                properties.insert("dns".into(), dns_value(family, &servers.join(";"))?);
            }
            if !search.is_empty() {
                properties.insert("dns-search".into(), owned(ZValue::from(search.clone())));
            }
        }
        profile.settings.insert(family.into(), properties);
    }
    Ok(profile)
}
