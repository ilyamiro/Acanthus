use std::collections::HashMap;
use std::path::Path;

use zbus::Variant as ZValue;

use super::{ImportError, ImportOptions, ImportedProfile, dict, dict_array, dns_value, invalid, owned, string_map};

const SERVICE_TYPE: &str = "org.freedesktop.NetworkManager.openvpn";
const INLINE_TAGS: &[&str] = &["ca", "cert", "key", "tls-auth", "tls-crypt", "secret"];
const QUIET: &[&str] = &[
    "client",
    "tls-client",
    "nobind",
    "persist-key",
    "persist-tun",
    "resolv-retry",
    "verb",
    "mute",
    "mute-replay-warnings",
    "auth-nocache",
    "pull",
    "float",
    "script-security",
    "keepalive",
];

pub(crate) fn looks_like_openvpn(text: &str) -> bool {
    let mut remote = false;
    let mut hint = false;
    for line in text.lines() {
        let first = line.split_whitespace().next().unwrap_or_default();
        match first {
            "remote" => remote = true,
            "client" | "tls-client" | "dev" | "proto" | "ca" | "auth-user-pass" | "<ca>" | "nobind" => hint = true,
            _ => {}
        }
    }
    remote && hint
}

fn words(line: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut current = String::new();
    let mut quoted = false;
    let mut any = false;
    for c in line.chars() {
        match c {
            '"' => {
                quoted = !quoted;
                any = true;
            }
            c if c.is_whitespace() && !quoted => {
                if any || !current.is_empty() {
                    out.push(std::mem::take(&mut current));
                    any = false;
                }
            }
            c => current.push(c),
        }
    }
    if any || !current.is_empty() {
        out.push(current);
    }
    out
}

fn resolve(options: &ImportOptions, path: &str) -> String {
    let candidate = Path::new(path);
    match &options.base_dir {
        Some(base) if candidate.is_relative() => base.join(candidate).to_string_lossy().into_owned(),
        _ => path.to_owned(),
    }
}

fn write_inline(options: &ImportOptions, name: &str, tag: &str, body: &str) -> Result<String, ImportError> {
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;
    let dir = options.output_dir.clone().ok_or(ImportError::Missing("output directory for inline certificates"))?;
    std::fs::create_dir_all(&dir)?;
    let safe: String = name.chars().map(|c| if c.is_ascii_alphanumeric() || c == '-' { c } else { '_' }).collect();
    let path = dir.join(format!("{safe}-{tag}.pem"));
    let mut file = std::fs::OpenOptions::new().write(true).create(true).truncate(true).mode(0o600).open(&path)?;
    file.write_all(body.trim().as_bytes())?;
    file.write_all(b"\n")?;
    Ok(path.to_string_lossy().into_owned())
}

/// Imports an OpenVPN configuration named `name`; inline certificates are written to the output directory of `options`.
pub fn import_openvpn(name: &str, text: &str, options: &ImportOptions) -> Result<ImportedProfile, ImportError> {
    let mut profile = ImportedProfile { name: name.to_owned(), ..Default::default() };
    let mut data: HashMap<String, String> = HashMap::new();
    let mut remotes: Vec<(String, Option<String>, Option<String>)> = Vec::new();
    let mut global_proto = "udp".to_owned();
    let mut global_port: Option<String> = None;
    let mut key_direction: Option<String> = None;
    let mut ta_direction: Option<String> = None;
    let mut user_pass = false;
    let mut static_key = false;
    let mut inline: Option<(String, String)> = None;
    let mut routes: Vec<ZValue<'static>> = Vec::new();
    let mut dns: Vec<String> = Vec::new();
    let mut search: Vec<String> = Vec::new();
    let mut redirect = false;
    let mut no_pull = false;

    for raw in text.lines() {
        if let Some((tag, body)) = inline.as_mut() {
            if raw.trim() == format!("</{tag}>") {
                let (tag, body) = inline.take().expect("inline block");
                let path = write_inline(options, name, &tag, &body)?;
                let key = match tag.as_str() {
                    "tls-auth" => "ta",
                    "secret" => "static-key",
                    other => other,
                };
                if tag == "secret" {
                    static_key = true;
                }
                data.insert(key.to_owned(), path);
            } else {
                body.push_str(raw);
                body.push('\n');
            }
            continue;
        }
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') || line.starts_with(';') {
            continue;
        }
        if let Some(tag) = line.strip_prefix('<').and_then(|l| l.strip_suffix('>'))
            && INLINE_TAGS.contains(&tag)
        {
            inline = Some((tag.to_owned(), String::new()));
            continue;
        }
        let parts = words(line);
        let Some(directive) = parts.first().map(String::as_str) else { continue };
        let args = &parts[1..];
        let arg = |n: usize| args.get(n).map(String::as_str);
        match directive {
            "remote" => {
                let host = arg(0).ok_or_else(|| invalid("remote", line))?;
                remotes.push((host.to_owned(), arg(1).map(str::to_owned), arg(2).map(str::to_owned)));
            }
            "proto" => global_proto = arg(0).unwrap_or("udp").to_owned(),
            "port" | "rport" => global_port = arg(0).map(str::to_owned),
            "dev" => {
                let dev = arg(0).ok_or_else(|| invalid("dev", line))?;
                let kind = if dev.starts_with("tap") { "tap" } else { "tun" };
                if dev != "tun" && dev != "tap" {
                    data.insert("dev".into(), dev.to_owned());
                }
                data.insert("dev-type".into(), kind.to_owned());
            }
            "ca" | "cert" | "key" => {
                let path = arg(0).ok_or_else(|| invalid(directive, line))?;
                data.insert(directive.to_owned(), resolve(options, path));
            }
            "tls-auth" => {
                let path = arg(0).ok_or_else(|| invalid(directive, line))?;
                data.insert("ta".into(), resolve(options, path));
                if let Some(direction) = arg(1) {
                    ta_direction = Some(direction.to_owned());
                }
            }
            "key-direction" => key_direction = arg(0).map(str::to_owned),
            "tls-crypt" | "tls-crypt-v2" => {
                let path = arg(0).ok_or_else(|| invalid(directive, line))?;
                data.insert(directive.to_owned(), resolve(options, path));
            }
            "secret" => {
                let path = arg(0).ok_or_else(|| invalid(directive, line))?;
                static_key = true;
                data.insert("static-key".into(), resolve(options, path));
                if let Some(direction) = arg(1) {
                    data.insert("static-key-direction".into(), direction.to_owned());
                }
            }
            "ifconfig" => {
                if let (Some(local), Some(remote)) = (arg(0), arg(1)) {
                    data.insert("local-ip".into(), local.to_owned());
                    data.insert("remote-ip".into(), remote.to_owned());
                }
            }
            "cipher" | "auth" | "remote-cert-tls" | "ns-cert-type" | "tls-version-min" | "tls-version-max"
            | "tls-cipher" | "connect-timeout" | "ping" | "ping-exit" | "ping-restart" => {
                let value = arg(0).ok_or_else(|| invalid(directive, line))?;
                data.insert(directive.to_owned(), value.to_owned());
            }
            "reneg-sec" => {
                data.insert("reneg-seconds".into(), arg(0).unwrap_or("0").to_owned());
            }
            "tun-mtu" => {
                data.insert("tunnel-mtu".into(), arg(0).unwrap_or_default().to_owned());
            }
            "fragment" => {
                data.insert("fragment-size".into(), arg(0).unwrap_or_default().to_owned());
            }
            "mssfix" => {
                data.insert("mssfix".into(), "yes".into());
                if let Some(size) = arg(0) {
                    data.insert("mssfix-size".into(), size.to_owned());
                }
            }
            "comp-lzo" => {
                let value = match arg(0) {
                    Some("no") => "no",
                    Some("adaptive") | None => "adaptive",
                    Some(_) => "yes",
                };
                data.insert("comp-lzo".into(), value.to_owned());
            }
            "compress" => {
                data.insert("compress".into(), arg(0).unwrap_or_default().to_owned());
            }
            "verify-x509-name" => {
                if let Some(subject) = arg(0) {
                    let kind = arg(1).unwrap_or("subject");
                    data.insert("verify-x509-name".into(), format!("{subject}:{kind}"));
                }
            }
            "http-proxy" | "socks-proxy" => {
                if let Some(server) = arg(0) {
                    let kind = if directive == "http-proxy" { "http" } else { "socks" };
                    data.insert("proxy-type".into(), kind.to_owned());
                    data.insert("proxy-server".into(), server.to_owned());
                    data.insert("proxy-port".into(), arg(1).unwrap_or("8080").to_owned());
                }
            }
            "auth-user-pass" => user_pass = true,
            "redirect-gateway" => redirect = true,
            "route-nopull" => no_pull = true,
            "pull-filter" => {
                if arg(0) == Some("ignore") && matches!(arg(1), Some("redirect-gateway" | "route")) {
                    no_pull = true;
                } else {
                    profile.ignored.push("pull-filter".to_owned());
                }
            }
            "route" => routes.push(route_from_directive(args).ok_or_else(|| invalid("route", line))?),
            "dhcp-option" => match (arg(0), arg(1)) {
                (Some("DNS"), Some(server)) => dns.push(server.to_owned()),
                (Some("DOMAIN" | "DOMAIN-SEARCH"), Some(domain)) => search.push(domain.to_owned()),
                (Some(option), _) => profile.ignored.push(format!("dhcp-option.{option}")),
                _ => profile.ignored.push("dhcp-option".to_owned()),
            },
            other if QUIET.contains(&other) => {}
            other => profile.ignored.push(other.to_owned()),
        }
    }
    if inline.is_some() {
        return Err(ImportError::Invalid { key: "inline block".into(), value: "unterminated".into() });
    }
    if remotes.is_empty() {
        return Err(ImportError::Missing("remote"));
    }

    let tcp = global_proto.starts_with("tcp");
    let rendered: Vec<String> = remotes
        .iter()
        .map(|(host, port, proto)| {
            let port = port.clone().or_else(|| global_port.clone()).unwrap_or_else(|| "1194".to_owned());
            let proto = proto.clone().unwrap_or_else(|| global_proto.clone());
            let proto = if proto.starts_with("tcp") { "tcp" } else { "udp" };
            format!("{host}:{port}:{proto}")
        })
        .collect();
    data.insert("remote".into(), rendered.join(", "));
    if tcp {
        data.insert("proto-tcp".into(), "yes".into());
    }
    if let Some(direction) = ta_direction.or(key_direction) {
        data.insert("ta-dir".into(), direction);
    }

    let has_cert = data.contains_key("cert") && data.contains_key("key");
    let kind = if static_key {
        "static-key"
    } else if user_pass && has_cert {
        "password-tls"
    } else if user_pass {
        "password"
    } else {
        "tls"
    };
    data.insert("connection-type".into(), kind.to_owned());
    if user_pass {
        data.insert("password-flags".into(), "1".into());
    }

    let entries: Vec<(String, String)> = {
        let mut sorted: Vec<_> = data.into_iter().collect();
        sorted.sort();
        sorted
    };
    profile.settings.insert(
        "connection".into(),
        HashMap::from([("id".to_owned(), owned(ZValue::from(name))), ("type".to_owned(), owned(ZValue::from("vpn")))]),
    );
    if !routes.is_empty() || !dns.is_empty() || !search.is_empty() || no_pull {
        let mut ipv4: HashMap<String, zbus::OwnedVariant> =
            HashMap::from([("method".to_owned(), owned(ZValue::from("auto")))]);
        if !routes.is_empty() {
            ipv4.insert("route-data".into(), owned(dict_array(routes)));
        }
        if !dns.is_empty() {
            ipv4.insert("dns".into(), dns_value("ipv4", &dns.join(";"))?);
        }
        if !search.is_empty() {
            ipv4.insert("dns-search".into(), owned(ZValue::from(search)));
        }
        if no_pull && !redirect {
            ipv4.insert("never-default".into(), owned(ZValue::from(true)));
        }
        profile.settings.insert("ipv4".into(), ipv4);
    }
    profile.settings.insert(
        "vpn".into(),
        HashMap::from([
            ("service-type".to_owned(), owned(ZValue::from(SERVICE_TYPE))),
            ("data".to_owned(), owned(string_map(&entries))),
        ]),
    );
    Ok(profile)
}

fn route_from_directive(args: &[String]) -> Option<ZValue<'static>> {
    let network = args.first()?;
    network.parse::<std::net::Ipv4Addr>().ok()?;
    let prefix = match args.get(1).map(String::as_str) {
        None | Some("default") => 32,
        Some(mask) => u32::from(mask.parse::<std::net::Ipv4Addr>().ok()?).count_ones(),
    };
    let mut fields: Vec<(&str, ZValue<'static>)> =
        vec![("dest", ZValue::from(network.clone())), ("prefix", ZValue::from(prefix))];
    if let Some(gateway) = args.get(2)
        && gateway.parse::<std::net::Ipv4Addr>().is_ok()
    {
        fields.push(("next-hop", ZValue::from(gateway.clone())));
    }
    if let Some(metric) = args.get(3).and_then(|m| m.parse::<u32>().ok()) {
        fields.push(("metric", ZValue::from(metric)));
    }
    Some(dict(fields))
}
