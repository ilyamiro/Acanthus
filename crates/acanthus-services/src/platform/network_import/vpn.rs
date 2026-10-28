use std::collections::HashMap;

use zbus::Variant as ZValue;

use super::{ImportError, ImportedProfile, invalid, owned, string_map};

fn vpn_profile(name: &str, plugin: &str, user: Option<&str>, data: Vec<(String, String)>) -> ImportedProfile {
    let mut data = data;
    data.sort();
    let mut vpn = HashMap::from([
        ("service-type".to_owned(), owned(ZValue::from(format!("org.freedesktop.NetworkManager.{plugin}")))),
        ("data".to_owned(), owned(string_map(&data))),
    ]);
    if let Some(user) = user {
        vpn.insert("user-name".into(), owned(ZValue::from(user)));
    }
    let mut profile = ImportedProfile { name: name.to_owned(), ..Default::default() };
    profile.settings.insert(
        "connection".into(),
        HashMap::from([("id".to_owned(), owned(ZValue::from(name))), ("type".to_owned(), owned(ZValue::from("vpn")))]),
    );
    profile.settings.insert("vpn".into(), vpn);
    profile
}

fn option_lines(text: &str) -> Vec<(String, Option<String>)> {
    text.lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#') && !l.starts_with(';'))
        .map(|line| {
            let equals = line.split_once('=').filter(|(key, _)| !key.trim().contains(char::is_whitespace));
            let split = equals.or_else(|| line.split_once(char::is_whitespace));
            match split {
                Some((key, value)) => {
                    (key.trim().to_ascii_lowercase(), Some(value.trim().trim_matches('"').to_owned()))
                }
                None => (line.to_ascii_lowercase(), None),
            }
        })
        .collect()
}

pub(crate) fn looks_like_openconnect(text: &str) -> bool {
    let lines = option_lines(text);
    let has = |k: &str| lines.iter().any(|(key, _)| key == k);
    has("server")
        && !has("remote")
        && ["protocol", "cafile", "servercert", "authgroup", "user", "certificate", "sslkey", "no-dtls", "useragent"]
            .iter()
            .any(|k| has(k))
}

pub fn import_openconnect(name: &str, text: &str) -> Result<ImportedProfile, ImportError> {
    let mut data: HashMap<String, String> = HashMap::new();
    let mut user = None;
    let mut ignored = Vec::new();
    for (key, value) in option_lines(text) {
        let flag = value.clone().unwrap_or_default();
        match key.as_str() {
            "server" => {
                data.insert("gateway".into(), flag);
            }
            "protocol" | "useragent" | "proxy" => {
                data.insert(key, flag);
            }
            "user" => user = value,
            "cafile" => {
                data.insert("cacert".into(), flag);
            }
            "certificate" => {
                data.insert("usercert".into(), flag);
            }
            "sslkey" => {
                data.insert("userkey".into(), flag);
            }
            "servercert" => {
                data.insert("gwcert".into(), flag);
            }
            "os" => {
                data.insert("reported_os".into(), flag);
            }
            "no-dtls" => {
                data.insert("disable_udp".into(), "yes".into());
            }
            "base-mtu" | "mtu" => {
                data.insert("mtu".into(), flag);
            }
            "csd-wrapper" => {
                data.insert("csd_wrapper".into(), flag);
                data.insert("enable_csd_trojan".into(), "yes".into());
            }
            "background" | "quiet" | "verbose" | "passwd-on-stdin" | "non-inter" | "syslog" | "timestamp" => {}
            other => ignored.push(other.to_owned()),
        }
    }
    if !data.contains_key("gateway") {
        return Err(ImportError::Missing("server"));
    }
    let cert = data.contains_key("usercert");
    data.insert("authtype".into(), if cert && user.is_none() { "cert" } else { "password" }.to_owned());
    if user.is_some() || !cert {
        data.insert("password-flags".into(), "1".into());
    }
    let mut profile = vpn_profile(name, "openconnect", user.as_deref(), data.into_iter().collect());
    profile.ignored = ignored;
    Ok(profile)
}

fn sections(text: &str, header: &str) -> Vec<(String, Vec<(String, String)>)> {
    let mut out: Vec<(String, Vec<(String, String)>)> = Vec::new();
    let mut open = false;
    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        let starts = |prefix: &str| trimmed.len() > prefix.len() && trimmed.to_ascii_lowercase().starts_with(prefix);
        let is_header = if header == "conn" {
            starts("conn ") && !line.starts_with(char::is_whitespace)
        } else {
            trimmed.starts_with('[')
        };
        if is_header {
            open = false;
            let title = if header == "conn" {
                trimmed[5..].trim().to_owned()
            } else {
                trimmed.trim_matches(|c| c == '[' || c == ']').trim().to_owned()
            };
            if (header == "conn" && title != "%default")
                || (header != "conn" && title.to_ascii_lowercase().starts_with(header))
            {
                out.push((title.trim_start_matches(header).trim().to_owned(), Vec::new()));
                open = true;
            }
        } else if open
            && let Some((key, value)) = trimmed.split_once('=')
            && let Some(last) = out.last_mut()
        {
            last.1.push((key.trim().to_ascii_lowercase(), value.trim().to_owned()));
        }
    }
    out
}

pub(crate) fn looks_like_strongswan(text: &str) -> bool {
    text.lines().any(|l| l.to_ascii_lowercase().starts_with("conn ") && !l.starts_with(char::is_whitespace))
        && text.contains("right")
}

pub fn import_strongswan(name_hint: &str, text: &str) -> Result<Vec<ImportedProfile>, ImportError> {
    let conns = sections(text, "conn");
    if conns.is_empty() {
        return Err(ImportError::Unrecognised);
    }
    let mut profiles = Vec::new();
    for (conn, entries) in conns {
        let get = |key: &str| entries.iter().find(|(k, _)| k == key).map(|(_, v)| v.clone());
        let address = get("right").ok_or(ImportError::Missing("right (gateway address)"))?;
        let mut data: HashMap<String, String> = HashMap::from([("address".to_owned(), address)]);
        let mut ignored = Vec::new();
        let eap = get("eap_identity").is_some() || get("leftauth").is_some_and(|a| a.starts_with("eap"));
        let psk =
            get("authby").is_some_and(|a| a == "secret" || a == "psk") || get("leftauth").is_some_and(|a| a == "psk");
        let method = if eap {
            "eap"
        } else if psk {
            "psk"
        } else {
            "key"
        };
        data.insert("method".into(), method.into());
        let user = get("eap_identity").or_else(|| if eap { get("leftid") } else { None });
        if let Some(id) = get("leftid") {
            data.insert("local-identity".into(), id);
        }
        if let Some(id) = get("rightid") {
            data.insert("remote-identity".into(), id);
        }
        if let Some(cert) = get("rightca").or_else(|| get("rightcert")) {
            data.insert("certificate".into(), cert);
        }
        if let Some(cert) = get("leftcert") {
            data.insert("usercert".into(), cert);
        }
        if get("leftsourceip").is_some_and(|v| v == "%config" || v == "%config4") {
            data.insert("virtual".into(), "yes".into());
        }
        if get("forceencaps").is_some_and(|v| v == "yes") {
            data.insert("encap".into(), "yes".into());
        }
        if get("ipcomp").is_some_and(|v| v == "yes") {
            data.insert("ipcomp".into(), "yes".into());
        }
        for key in ["ike", "esp"] {
            if let Some(value) = get(key) {
                let strict = value.ends_with('!');
                data.insert(key.into(), value.trim_end_matches('!').to_owned());
                if strict {
                    data.insert("proposal".into(), "yes".into());
                }
            }
        }
        if method != "key" {
            data.insert("password-flags".into(), "1".into());
        }
        const HANDLED: &[&str] = &[
            "right",
            "rightid",
            "rightca",
            "rightcert",
            "leftcert",
            "leftid",
            "leftauth",
            "authby",
            "eap_identity",
            "leftsourceip",
            "forceencaps",
            "ipcomp",
            "ike",
            "esp",
            "keyexchange",
            "auto",
            "type",
            "left",
            "rightsubnet",
            "leftsubnet",
            "rightauth",
            "leftfirewall",
        ];
        ignored.extend(entries.iter().filter(|(k, _)| !HANDLED.contains(&k.as_str())).map(|(k, _)| k.clone()));
        let title = if conn.is_empty() { name_hint.to_owned() } else { conn };
        let mut profile = vpn_profile(&title, "strongswan", user.as_deref(), data.into_iter().collect());
        profile.ignored = ignored;
        profiles.push(profile);
    }
    Ok(profiles)
}

const PPP_FLAGS: &[(&str, &str)] = &[
    ("refuse-pap", "refuse-pap"),
    ("refuse-chap", "refuse-chap"),
    ("refuse-mschap", "refuse-mschap"),
    ("refuse-mschap-v2", "refuse-mschapv2"),
    ("refuse-mschapv2", "refuse-mschapv2"),
    ("refuse-eap", "refuse-eap"),
    ("require-mppe", "require-mppe"),
    ("require-mppe-40", "require-mppe-40"),
    ("require-mppe-128", "require-mppe-128"),
    ("mppe-stateful", "mppe-stateful"),
    ("nobsdcomp", "nobsdcomp"),
    ("nodeflate", "nodeflate"),
    ("novj", "no-vj-comp"),
    ("no-vj-comp", "no-vj-comp"),
];

const PPP_VALUES: &[&str] = &["mtu", "mru", "lcp-echo-failure", "lcp-echo-interval"];

fn ppp_options(text: &str, data: &mut HashMap<String, String>) {
    for (key, value) in option_lines(text) {
        let key = key.replace(' ', "-");
        if let Some((_, target)) = PPP_FLAGS.iter().find(|(source, _)| *source == key) {
            data.insert((*target).to_owned(), "yes".to_owned());
        } else if PPP_VALUES.contains(&key.as_str())
            && let Some(value) = value
        {
            data.insert(key, value);
        }
    }
}

pub(crate) fn looks_like_l2tp(text: &str) -> bool {
    text.lines().any(|l| l.trim().to_ascii_lowercase().starts_with("[lac "))
}

pub fn import_l2tp(name_hint: &str, text: &str) -> Result<Vec<ImportedProfile>, ImportError> {
    let lacs = sections(text, "lac");
    if lacs.is_empty() {
        return Err(ImportError::Unrecognised);
    }
    let mut profiles = Vec::new();
    for (title, entries) in lacs {
        let get = |key: &str| entries.iter().find(|(k, _)| k == key).map(|(_, v)| v.clone());
        let gateway = get("lns").ok_or(ImportError::Missing("lns (gateway)"))?;
        let mut data = HashMap::from([("gateway".to_owned(), gateway)]);
        let flags: String = entries
            .iter()
            .filter(|(k, val)| {
                (k.starts_with("refuse ") || k.starts_with("require ")) && val.eq_ignore_ascii_case("yes")
            })
            .map(|(k, _)| format!("{}\n", k.replace(' ', "-")))
            .collect();
        ppp_options(&flags, &mut data);
        ppp_options(text, &mut data);
        data.insert("password-flags".into(), "1".into());
        let user = get("name");
        let name = if title.is_empty() { name_hint.to_owned() } else { title };
        let mut profile = vpn_profile(&name, "l2tp", user.as_deref(), data.into_iter().collect());
        profile.ignored = entries
            .iter()
            .filter(|(k, _)| {
                !matches!(k.as_str(), "lns" | "name") && !k.starts_with("refuse ") && !k.starts_with("require ")
            })
            .map(|(k, _)| k.clone())
            .collect();
        profiles.push(profile);
    }
    Ok(profiles)
}

pub(crate) fn looks_like_pptp(text: &str) -> bool {
    let lower = text.to_ascii_lowercase();
    lower.contains("pptp") && lower.lines().any(|l| l.trim_start().starts_with("pty"))
}

pub fn import_pptp(name_hint: &str, text: &str) -> Result<ImportedProfile, ImportError> {
    let mut gateway = None;
    let mut user = None;
    let mut ignored = Vec::new();
    for (key, value) in option_lines(text) {
        match key.as_str() {
            "pty" => {
                let command = value.unwrap_or_default();
                let mut words = command.split_whitespace();
                if words.next().is_some_and(|w| w.ends_with("pptp")) {
                    gateway = words.next().map(str::to_owned);
                }
            }
            "name" => user = value,
            _ => {
                let known = PPP_FLAGS.iter().any(|(s, _)| *s == key) || PPP_VALUES.contains(&key.as_str());
                if !known {
                    ignored.push(key);
                }
            }
        }
    }
    let gateway = gateway.ok_or_else(|| invalid("pty", "no pptp server"))?;
    let mut data = HashMap::from([("gateway".to_owned(), gateway), ("password-flags".to_owned(), "1".to_owned())]);
    ppp_options(text, &mut data);
    let mut profile = vpn_profile(name_hint, "pptp", user.as_deref(), data.into_iter().collect());
    profile.ignored = ignored;
    Ok(profile)
}
