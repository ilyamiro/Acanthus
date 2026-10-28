use std::sync::Mutex;

use serde_json::{Map, Number, Value as Json};
use zbus::{Connection, Variant};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BusKind {
    System,
    Session,
    Address(String),
}

impl BusKind {
    pub async fn connect(&self) -> zbus::Result<Connection> {
        match self {
            BusKind::System => Connection::system().await,
            BusKind::Session => Connection::session().await,
            BusKind::Address(address) => zbus::connection::Builder::address(address.as_str()).build().await,
        }
    }
}

static SHARED_SYSTEM: Mutex<Option<Connection>> = Mutex::new(None);
static SHARED_SESSION: Mutex<Option<Connection>> = Mutex::new(None);

fn shared_slot(kind: &BusKind) -> Option<&'static Mutex<Option<Connection>>> {
    match kind {
        BusKind::System => Some(&SHARED_SYSTEM),
        BusKind::Session => Some(&SHARED_SESSION),
        BusKind::Address(_) => None,
    }
}

impl BusKind {
    pub async fn connect_shared(&self) -> zbus::Result<Connection> {
        let Some(slot) = shared_slot(self) else {
            return self.connect().await;
        };
        if let Some(connection) = slot.lock().unwrap_or_else(|e| e.into_inner()).clone() {
            return Ok(connection);
        }
        let connection = self.connect().await?;
        let mut guard = slot.lock().unwrap_or_else(|e| e.into_inner());
        Ok(guard.get_or_insert(connection).clone())
    }

    pub fn forget_shared(&self) {
        if let Some(slot) = shared_slot(self) {
            slot.lock().unwrap_or_else(|e| e.into_inner()).take();
        }
    }
}

pub fn forget_all_shared() {
    BusKind::System.forget_shared();
    BusKind::Session.forget_shared();
}

pub fn variant_to_json(value: &Variant<'_>) -> Json {
    let number = |n: f64| Number::from_f64(n).map_or(Json::Null, Json::Number);
    match value {
        Variant::U8(n) => Json::from(*n),
        Variant::Bool(b) => Json::Bool(*b),
        Variant::I16(n) => Json::from(*n),
        Variant::U16(n) => Json::from(*n),
        Variant::I32(n) => Json::from(*n),
        Variant::U32(n) => Json::from(*n),
        Variant::I64(n) => Json::from(*n),
        Variant::U64(n) => Json::from(*n),
        Variant::F64(n) => number(*n),
        Variant::Str(s) => Json::String(s.to_string()),
        Variant::Signature(s) => Json::String(s.to_string()),
        Variant::ObjectPath(p) => Json::String(p.to_string()),
        Variant::Variant(inner) => variant_to_json(inner),
        Variant::Array(array) => Json::Array(array.inner().iter().map(variant_to_json).collect()),
        Variant::Dict(dict) => {
            let mut map = Map::new();
            for (key, entry) in dict.iter() {
                let key = match variant_to_json(key) {
                    Json::String(s) => s,
                    other => other.to_string(),
                };
                map.insert(key, variant_to_json(entry));
            }
            Json::Object(map)
        }
        Variant::Structure(structure) => Json::Array(structure.fields().iter().map(variant_to_json).collect()),
        _ => Json::Null,
    }
}

#[cfg(test)]
pub mod testbus {
    use std::io::{BufRead, BufReader};
    use std::process::{Child, Command, Stdio};

    use super::BusKind;

    const CONFIG: &str = r#"<busconfig>
  <type>session</type>
  <listen>unix:tmpdir=/tmp</listen>
  <auth>EXTERNAL</auth>
  <policy context="default">
    <allow send_destination="*" eavesdrop="true"/>
    <allow eavesdrop="true"/>
    <allow own="*"/>
  </policy>
</busconfig>
"#;

    pub struct TestBus {
        child: Child,
        pub address: String,
        _dir: tempfile::TempDir,
    }

    impl TestBus {
        pub fn start() -> TestBus {
            let dir = tempfile::tempdir().unwrap();
            let config = dir.path().join("bus.conf");
            std::fs::write(&config, CONFIG).unwrap();
            let mut child = Command::new("dbus-daemon")
                .arg(format!("--config-file={}", config.display()))
                .arg("--nofork")
                .arg("--print-address=1")
                .stdout(Stdio::piped())
                .stderr(Stdio::null())
                .spawn()
                .expect("dbus-daemon is required for service tests");
            let mut line = String::new();
            BufReader::new(child.stdout.take().unwrap()).read_line(&mut line).unwrap();
            TestBus { child, address: line.trim().to_owned(), _dir: dir }
        }

        pub fn kind(&self) -> BusKind {
            BusKind::Address(self.address.clone())
        }
    }

    impl Drop for TestBus {
        fn drop(&mut self) {
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use serde_json::json;
    use zbus::OwnedVariant;

    use super::*;

    #[test]
    fn shared_connection_is_reused_until_forgotten() {
        let bus = BusKind::Session;
        assert!(shared_slot(&bus).is_some());
        assert!(shared_slot(&BusKind::Address("unix:path=/nonexistent".into())).is_none());
        bus.forget_shared();
        assert!(SHARED_SESSION.lock().unwrap().is_none());
    }

    #[test]
    fn nested_variants_become_plain_json() {
        let mut inner: HashMap<String, OwnedVariant> = HashMap::new();
        inner.insert("title".into(), OwnedVariant::try_from(Variant::from("Song")).unwrap());
        inner.insert(
            "artists".into(),
            OwnedVariant::try_from(Variant::from(vec!["A".to_owned(), "B".to_owned()])).unwrap(),
        );
        inner.insert("length".into(), OwnedVariant::try_from(Variant::from(5i64)).unwrap());
        inner.insert("ok".into(), OwnedVariant::try_from(Variant::from(true)).unwrap());
        let value = OwnedVariant::try_from(Variant::from(inner)).unwrap();
        assert_eq!(variant_to_json(&value), json!({"title": "Song", "artists": ["A", "B"], "length": 5, "ok": true}));
    }
}
