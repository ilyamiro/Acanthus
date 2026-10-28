use std::sync::Mutex;

use futures_lite::{FutureExt, StreamExt};
use serde_json::{Map, Number, Value as Json};
use zbus::fdo::DBusProxy;
use zbus::names::BusName;
use zbus::{Connection, Variant};

/// Which message bus to connect to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BusKind {
    /// The system bus.
    System,
    /// The session bus.
    Session,
    /// A bus at an explicit address, used in tests.
    Address(String),
}

impl BusKind {
    /// Opens a new connection.
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
    /// Returns the process-wide connection to the system or session bus, opening it on first use; other kinds always connect anew.
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

    /// Drops the shared connection so the next call reconnects.
    pub fn forget_shared(&self) {
        if let Some(slot) = shared_slot(self) {
            slot.lock().unwrap_or_else(|e| e.into_inner()).take();
        }
    }
}

/// Drops both shared connections.
pub fn forget_all_shared() {
    BusKind::System.forget_shared();
    BusKind::Session.forget_shared();
}

/// Runs `body` as long as the well-known `name` keeps the owner it had when the call started.
///
/// When the service behind `name` exits, restarts or is replaced, `body` is dropped and the call
/// fails, so a supervised worker reconnects and reads the state of the new owner from scratch
/// instead of keeping the old one.
pub async fn while_owned<T>(
    connection: &Connection,
    name: &str,
    body: impl Future<Output = zbus::Result<T>>,
) -> zbus::Result<T> {
    let dbus = DBusProxy::new(connection).await?;
    let mut changes = dbus.receive_name_owner_changed_with_args(&[(0, name)]).await?;
    let changed = async {
        changes.next().await;
        Err(zbus::Error::Failure(format!("{name} changed owner")))
    };
    body.or(changed).await
}

/// Waits until one of the well-known `names` has an owner; returns at once when one has.
pub async fn wait_for_owner(connection: &Connection, names: &[&str]) -> zbus::Result<()> {
    let dbus = DBusProxy::new(connection).await?;
    let mut changes = dbus.receive_name_owner_changed().await?;
    for name in names {
        if dbus.name_has_owner(BusName::try_from(*name)?).await? {
            return Ok(());
        }
    }
    while let Some(change) = changes.next().await {
        let Ok(args) = change.args() else { continue };
        if args.new_owner().is_some() && names.contains(&args.name().as_str()) {
            return Ok(());
        }
    }
    Ok(())
}

/// Converts a D-Bus value to JSON; dictionaries become objects and unsupported values become null.
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

/// A private message bus for tests; needs `dbus-daemon` installed.
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

    /// A running private message bus; it stops when dropped.
    pub struct TestBus {
        child: Child,
        /// The address to connect to.
        pub address: String,
        _dir: tempfile::TempDir,
    }

    impl TestBus {
        /// Starts a private bus and waits for its address.
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

        /// The bus as a [`BusKind`].
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

    const WATCHED: &str = "org.example.Watched";

    async fn owner(bus: &BusKind) -> Connection {
        let connection = bus.connect().await.unwrap();
        connection.request_name(WATCHED).await.unwrap();
        connection
    }

    async fn within<T>(seconds: u64, work: impl Future<Output = T>) -> Option<T> {
        async { Some(work.await) }
            .or(async {
                async_io::Timer::after(std::time::Duration::from_secs(seconds)).await;
                None
            })
            .await
    }

    #[test]
    fn a_body_stops_when_its_service_changes_owner() {
        let test_bus = testbus::TestBus::start();
        async_io::block_on(async {
            let bus = test_bus.kind();
            let service = owner(&bus).await;
            let client = bus.connect().await.unwrap();
            let watched = while_owned(&client, WATCHED, std::future::pending::<zbus::Result<()>>());
            let stopper = async {
                async_io::Timer::after(std::time::Duration::from_millis(100)).await;
                drop(service);
                std::future::pending::<zbus::Result<()>>().await
            };
            let result = within(5, watched.or(stopper)).await.expect("the owner change was not noticed");
            assert!(matches!(result, Err(zbus::Error::Failure(message)) if message.contains(WATCHED)));
            assert_eq!(within(5, while_owned(&client, WATCHED, async { Ok(7) })).await.unwrap().unwrap(), 7);
        });
    }

    #[test]
    fn waiting_for_an_owner_ends_when_the_service_appears() {
        let test_bus = testbus::TestBus::start();
        async_io::block_on(async {
            let bus = test_bus.kind();
            let client = bus.connect().await.unwrap();
            let names = ["org.example.Other", WATCHED];
            let appear = async {
                async_io::Timer::after(std::time::Duration::from_millis(100)).await;
                let service = owner(&bus).await;
                std::future::pending::<zbus::Result<()>>().await?;
                drop(service);
                Ok(())
            };
            let waited = within(5, wait_for_owner(&client, &names).or(appear)).await;
            assert!(matches!(waited, Some(Ok(()))));
            let _service = owner(&bus).await;
            assert!(matches!(within(5, wait_for_owner(&client, &names)).await, Some(Ok(()))));
        });
    }

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
