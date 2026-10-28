use std::collections::HashMap;

use zbus::{OwnedVariant as OwnedValue, Variant as ZValue};

use super::{ImportError, ImportedProfile, cert_value, invalid, owned, parse_hex, split_list};

struct Value {
    text: String,
    quoted: bool,
}

fn parse_blocks(text: &str) -> Vec<Vec<(String, Value)>> {
    let mut blocks = Vec::new();
    let mut current: Option<Vec<(String, Value)>> = None;
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let compact: String = line.chars().filter(|c| !c.is_whitespace()).collect();
        if current.is_none() {
            if compact == "network={" {
                current = Some(Vec::new());
            }
            continue;
        }
        if line == "}" {
            blocks.extend(current.take());
            continue;
        }
        if let Some((key, value)) = line.split_once('=') {
            let value = value.trim();
            let (text, quoted) = match value.strip_prefix('"').and_then(|v| v.strip_suffix('"')) {
                Some(inner) => (inner.to_owned(), true),
                None => (value.to_owned(), false),
            };
            if let Some(block) = current.as_mut() {
                block.push((key.trim().to_ascii_lowercase(), Value { text, quoted }));
            }
        }
    }
    blocks
}

fn ssid_bytes(value: &Value) -> Result<Vec<u8>, ImportError> {
    if value.quoted { Ok(value.text.as_bytes().to_vec()) } else { parse_hex("ssid", &value.text) }
}

fn lower_list(value: &str) -> Vec<String> {
    value.split_whitespace().map(str::to_ascii_lowercase).collect()
}

fn eap_name(method: &str) -> Option<&'static str> {
    match method.to_ascii_uppercase().as_str() {
        "PEAP" => Some("peap"),
        "TTLS" => Some("ttls"),
        "TLS" => Some("tls"),
        "FAST" => Some("fast"),
        "PWD" => Some("pwd"),
        "SIM" => Some("sim"),
        "AKA" => Some("aka"),
        "AKA'" => Some("aka'"),
        _ => None,
    }
}

fn convert_block(block: &[(String, Value)]) -> Result<ImportedProfile, ImportError> {
    let get = |key: &str| block.iter().find(|(k, _)| k == key).map(|(_, v)| v);
    let ssid_value = get("ssid").ok_or(ImportError::Missing("ssid"))?;
    let ssid = ssid_bytes(ssid_value)?;
    let name = String::from_utf8_lossy(&ssid).into_owned();
    let mut profile = ImportedProfile { name: name.clone(), ..Default::default() };

    let mut connection = HashMap::from([
        ("id".to_owned(), owned(ZValue::from(name.as_str()))),
        ("type".to_owned(), owned(ZValue::from("802-11-wireless"))),
    ]);
    if let Some(priority) = get("priority") {
        let n = priority.text.parse::<i32>().map_err(|_| invalid("priority", &priority.text))?;
        connection.insert("autoconnect-priority".into(), owned(ZValue::from(n)));
    }
    if get("disabled").is_some_and(|v| v.text == "1") {
        connection.insert("autoconnect".into(), owned(ZValue::from(false)));
    }

    let mut wireless = HashMap::from([
        ("ssid".to_owned(), owned(ZValue::from(ssid))),
        ("mode".to_owned(), owned(ZValue::from("infrastructure"))),
    ]);
    if let Some(mode) = get("mode") {
        let mode = match mode.text.as_str() {
            "1" => "adhoc",
            "2" => "ap",
            _ => "infrastructure",
        };
        wireless.insert("mode".into(), owned(ZValue::from(mode)));
    }
    if get("scan_ssid").is_some_and(|v| v.text == "1") {
        wireless.insert("hidden".into(), owned(ZValue::from(true)));
    }
    if let Some(bssid) = get("bssid") {
        wireless.insert("bssid".into(), owned(ZValue::from(super::parse_mac("bssid", &bssid.text)?)));
    }

    let key_mgmt_list = get("key_mgmt").map(|v| lower_list(&v.text)).unwrap_or_default();
    let has = |name: &str| key_mgmt_list.iter().any(|k| k == name);
    let key_mgmt = if has("wpa-eap") || has("ieee8021x") || has("wpa-eap-sha256") {
        Some("wpa-eap")
    } else if has("wpa-psk") || has("wpa-psk-sha256") {
        Some("wpa-psk")
    } else if has("sae") {
        Some("sae")
    } else if has("owe") {
        Some("owe")
    } else if get("wep_key0").is_some() {
        Some("none")
    } else {
        None
    };
    for item in &key_mgmt_list {
        if !matches!(
            item.as_str(),
            "wpa-eap" | "ieee8021x" | "wpa-eap-sha256" | "wpa-psk" | "wpa-psk-sha256" | "sae" | "owe" | "none"
        ) {
            profile.ignored.push(format!("key_mgmt.{item}"));
        }
    }

    let mut security: HashMap<String, OwnedValue> = HashMap::new();
    if let Some(key_mgmt) = key_mgmt {
        security.insert("key-mgmt".into(), owned(ZValue::from(key_mgmt)));
    }
    for (source, target) in [("proto", "proto"), ("pairwise", "pairwise"), ("group", "group")] {
        if let Some(value) = get(source) {
            security.insert(target.into(), owned(ZValue::from(lower_list(&value.text))));
        }
    }
    if let Some(psk) = get("psk") {
        let valid = if psk.quoted { (8..=63).contains(&psk.text.len()) } else { psk.text.len() == 64 };
        if !valid {
            return Err(invalid("psk", &psk.text));
        }
        security.insert("psk".into(), owned(ZValue::from(psk.text.as_str())));
    }
    if let Some(pmf) = get("ieee80211w") {
        let value = match pmf.text.as_str() {
            "0" => 1i32,
            "1" => 2,
            "2" => 3,
            other => return Err(invalid("ieee80211w", other)),
        };
        security.insert("pmf".into(), owned(ZValue::from(value)));
    }
    if let Some(auth) = get("auth_alg") {
        let alg = match auth.text.to_ascii_uppercase().as_str() {
            "SHARED" => "shared",
            "LEAP" => "leap",
            _ => "open",
        };
        security.insert("auth-alg".into(), owned(ZValue::from(alg)));
    }
    if let Some(key) = get("wep_key0") {
        if !key.quoted {
            parse_hex("wep_key0", &key.text)?;
        }
        security.insert("wep-key0".into(), owned(ZValue::from(key.text.as_str())));
        security.insert("wep-key-type".into(), owned(ZValue::from(1u32)));
        if let Some(index) = get("wep_tx_keyidx") {
            let n = index.text.parse::<u32>().map_err(|_| invalid("wep_tx_keyidx", &index.text))?;
            security.insert("wep-tx-keyidx".into(), owned(ZValue::from(n)));
        }
    }

    let mut eap_settings: HashMap<String, OwnedValue> = HashMap::new();
    if let Some(methods) = get("eap") {
        let methods: Vec<&'static str> = methods.text.split_whitespace().filter_map(eap_name).collect();
        if methods.is_empty() {
            return Err(invalid("eap", &methods_text(get("eap"))));
        }
        eap_settings
            .insert("eap".into(), owned(ZValue::from(methods.iter().map(|m| (*m).to_owned()).collect::<Vec<_>>())));
        security.entry("key-mgmt".into()).or_insert_with(|| owned(ZValue::from("wpa-eap")));
    }
    let text_fields = [
        ("identity", "identity"),
        ("anonymous_identity", "anonymous-identity"),
        ("password", "password"),
        ("private_key_passwd", "private-key-password"),
        ("subject_match", "subject-match"),
        ("domain_suffix_match", "domain-suffix-match"),
        ("domain_match", "domain-match"),
        ("pac_file", "pac-file"),
        ("pin", "pin"),
        ("ca_path", "ca-path"),
        ("subject_match2", "phase2-subject-match"),
        ("domain_suffix_match2", "phase2-domain-suffix-match"),
    ];
    for (source, target) in text_fields {
        if let Some(value) = get(source) {
            if source == "password" && value.text.starts_with("hash:") {
                profile.ignored.push("password.hash".into());
                continue;
            }
            eap_settings.insert(target.into(), owned(ZValue::from(value.text.as_str())));
        }
    }
    let cert_fields = [
        ("ca_cert", "ca-cert"),
        ("client_cert", "client-cert"),
        ("private_key", "private-key"),
        ("ca_cert2", "phase2-ca-cert"),
        ("client_cert2", "phase2-client-cert"),
        ("private_key2", "phase2-private-key"),
    ];
    for (source, target) in cert_fields {
        if let Some(value) = get(source) {
            eap_settings.insert(target.into(), owned(ZValue::from(cert_value(&value.text))));
        }
    }
    if let Some(value) = get("altsubject_match") {
        eap_settings.insert("altsubject-matches".into(), owned(ZValue::from(split_list(&value.text, ';'))));
    }
    if let Some(value) = get("phase1") {
        for token in value.text.split_whitespace() {
            match token.split_once('=') {
                Some(("peapver", v)) => insert_text(&mut eap_settings, "phase1-peapver", v),
                Some(("peaplabel", v)) => insert_text(&mut eap_settings, "phase1-peaplabel", v),
                Some(("fast_provisioning", v)) => insert_text(&mut eap_settings, "phase1-fast-provisioning", v),
                _ => profile.ignored.push(format!("phase1.{token}")),
            }
        }
    }
    if let Some(value) = get("phase2") {
        for token in value.text.split_whitespace() {
            match token.split_once('=') {
                Some(("auth", v)) => insert_text(&mut eap_settings, "phase2-auth", &v.to_ascii_lowercase()),
                Some(("autheap", v)) => insert_text(&mut eap_settings, "phase2-autheap", &v.to_ascii_lowercase()),
                _ => profile.ignored.push(format!("phase2.{token}")),
            }
        }
    }

    const HANDLED: &[&str] = &[
        "ssid",
        "priority",
        "disabled",
        "mode",
        "scan_ssid",
        "bssid",
        "key_mgmt",
        "proto",
        "pairwise",
        "group",
        "psk",
        "ieee80211w",
        "auth_alg",
        "wep_key0",
        "wep_tx_keyidx",
        "eap",
        "identity",
        "anonymous_identity",
        "password",
        "private_key_passwd",
        "subject_match",
        "domain_suffix_match",
        "domain_match",
        "pac_file",
        "pin",
        "ca_path",
        "subject_match2",
        "domain_suffix_match2",
        "ca_cert",
        "client_cert",
        "private_key",
        "ca_cert2",
        "client_cert2",
        "private_key2",
        "altsubject_match",
        "phase1",
        "phase2",
    ];
    for (key, _) in block {
        if !HANDLED.contains(&key.as_str()) {
            profile.ignored.push(key.clone());
        }
    }

    profile.settings.insert("connection".into(), connection);
    profile.settings.insert("802-11-wireless".into(), wireless);
    if !security.is_empty() {
        profile
            .settings
            .get_mut("802-11-wireless")
            .expect("just inserted")
            .insert("security".into(), owned(ZValue::from("802-11-wireless-security")));
        profile.settings.insert("802-11-wireless-security".into(), security);
    }
    if !eap_settings.is_empty() {
        profile.settings.insert("802-1x".into(), eap_settings);
    }
    Ok(profile)
}

fn methods_text(value: Option<&Value>) -> String {
    value.map(|v| v.text.clone()).unwrap_or_default()
}

fn insert_text(map: &mut HashMap<String, OwnedValue>, key: &str, value: &str) {
    map.insert(key.to_owned(), owned(ZValue::from(value)));
}

/// Imports the networks of a wpa_supplicant configuration, one profile per network.
pub fn import_wpa_supplicant(text: &str) -> Result<Vec<ImportedProfile>, ImportError> {
    let blocks = parse_blocks(text);
    if blocks.is_empty() {
        return Err(ImportError::Unrecognised);
    }
    blocks.iter().map(|block| convert_block(block)).collect()
}
