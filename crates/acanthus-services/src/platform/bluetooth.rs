use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use acanthus_core::{Signal, batch};
use acanthus_io::IoError;
use calloop::LoopHandle;
use futures_lite::StreamExt;
use zbus::Connection;
use zbus::{OwnedVariant as OwnedValue, Variant as ZValue};

use crate::platform::bridge::{EventSink, ServiceHandle, spawn_supervised};
use crate::platform::dbus::BusKind;

const SERVICE: &str = "org.bluez";
const ADAPTER_IFACE: &str = "org.bluez.Adapter1";
const DEVICE_IFACE: &str = "org.bluez.Device1";
const BATTERY_IFACE: &str = "org.bluez.Battery1";
const AGENT_MANAGER_IFACE: &str = "org.bluez.AgentManager1";
fn agent_path(app: &str) -> String {
    format!("/org/{app}/BluezAgent")
}
const AGENT_CAPABILITY: &str = "KeyboardDisplay";

pub type Properties = HashMap<String, OwnedValue>;
pub type Objects = HashMap<String, HashMap<String, Properties>>;

#[derive(Debug, Clone, PartialEq, Default)]
pub struct BluetoothDevice {
    pub path: String,
    pub address: String,
    pub name: String,
    pub alias: String,
    pub icon: String,
    pub paired: bool,
    pub trusted: bool,
    pub connected: bool,
    pub battery_available: bool,
    pub battery: f64,
}

impl BluetoothDevice {
    pub fn display_name(&self) -> &str {
        [&self.name, &self.alias, &self.address].into_iter().find(|s| !s.is_empty()).map_or("", |s| s.as_str())
    }
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct BluetoothSnapshot {
    pub adapter_path: Option<String>,
    pub adapter_name: String,
    pub enabled: bool,
    pub discovering: bool,
    pub devices: Vec<BluetoothDevice>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum BluetoothCommand {
    SetPowered(bool),
    SetDiscovering(bool),
    Connect(String),
    Disconnect(String),
    Pair(String),
    SetTrusted(String, bool),
    Forget(String),
    PairingAnswer { id: u64, answer: Option<PairingAnswer> },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PairingKind {
    PinCode,
    Passkey,
    Confirmation { passkey: u32 },
    Authorization,
    AuthorizeService { uuid: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PairingRequest {
    pub id: u64,
    pub device_path: String,
    pub device_name: String,
    pub kind: PairingKind,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PairingDisplay {
    pub device_path: String,
    pub device_name: String,
    pub code: String,
    pub entered: u16,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PairingAnswer {
    Pin(String),
    Passkey(u32),
    Accept,
    Reject,
}

type PairingPending = Arc<Mutex<HashMap<u64, async_channel::Sender<Option<PairingAnswer>>>>>;

#[derive(Debug, zbus::DBusError)]
#[zbus(prefix = "org.bluez.Error")]
enum AgentError {
    #[zbus(error)]
    ZBus(zbus::Error),
    Rejected(String),
    Canceled(String),
}

struct PairingAgent {
    connection: Connection,
    events: EventSink<Event>,
    pending: PairingPending,
    next_id: u64,
}

impl PairingAgent {
    async fn device_info(&self, device: &str) -> (String, bool) {
        let read = async {
            let properties = zbus::fdo::PropertiesProxy::builder(&self.connection)
                .destination(SERVICE)
                .path(device.to_owned())
                .build()
                .await?;
            let all = properties.get_all(DEVICE_IFACE.try_into()?).await?;
            let text = |key: &str| all.get(key).and_then(as_str).unwrap_or_default();
            let flag = |key: &str| all.get(key).and_then(as_bool).unwrap_or(false);
            let name = [text("Alias"), text("Name"), text("Address")].into_iter().find(|s| !s.is_empty());
            zbus::Result::Ok((name.unwrap_or_default(), flag("Paired") || flag("Trusted")))
        };
        read.await.unwrap_or_default()
    }

    async fn ask(&mut self, device: &str, kind: PairingKind) -> Option<PairingAnswer> {
        let (name, _) = self.device_info(device).await;
        self.next_id += 1;
        let id = self.next_id;
        let (tx, rx) = async_channel::bounded(1);
        self.pending.lock().expect("pairing lock").insert(id, tx);
        self.events.send(Event::PairingRequested(PairingRequest {
            id,
            device_path: device.to_owned(),
            device_name: name,
            kind,
        }));
        let answer = rx.recv().await.ok().flatten();
        self.pending.lock().expect("pairing lock").remove(&id);
        self.events.send(Event::PairingFinished(id));
        answer
    }

    async fn show(&self, device: &str, code: String, entered: u16) {
        let (name, _) = self.device_info(device).await;
        self.events.send(Event::PairingDisplay(Some(PairingDisplay {
            device_path: device.to_owned(),
            device_name: name,
            code,
            entered,
        })));
    }

    fn cancel_all(&self) {
        let waiting: Vec<_> = self.pending.lock().expect("pairing lock").drain().collect();
        for (id, tx) in waiting {
            let _ = tx.try_send(None);
            self.events.send(Event::PairingFinished(id));
        }
        self.events.send(Event::PairingDisplay(None));
    }
}

#[zbus::interface(name = "org.bluez.Agent1")]
impl PairingAgent {
    async fn release(&mut self) {
        self.cancel_all();
    }

    async fn request_pin_code(&mut self, device: zbus::ObjectPath<'_>) -> Result<String, AgentError> {
        match self.ask(device.as_str(), PairingKind::PinCode).await {
            Some(PairingAnswer::Pin(pin)) => Ok(pin),
            Some(PairingAnswer::Reject) => Err(AgentError::Rejected("rejected".into())),
            _ => Err(AgentError::Canceled("canceled".into())),
        }
    }

    async fn display_pin_code(&mut self, device: zbus::ObjectPath<'_>, pincode: String) {
        self.show(device.as_str(), pincode, 0).await;
    }

    async fn request_passkey(&mut self, device: zbus::ObjectPath<'_>) -> Result<u32, AgentError> {
        match self.ask(device.as_str(), PairingKind::Passkey).await {
            Some(PairingAnswer::Passkey(key)) => Ok(key),
            Some(PairingAnswer::Pin(text)) => text.trim().parse().map_err(|_| AgentError::Rejected("invalid".into())),
            Some(PairingAnswer::Reject) => Err(AgentError::Rejected("rejected".into())),
            _ => Err(AgentError::Canceled("canceled".into())),
        }
    }

    async fn display_passkey(&mut self, device: zbus::ObjectPath<'_>, passkey: u32, entered: u16) {
        self.show(device.as_str(), format!("{passkey:06}"), entered).await;
    }

    async fn request_confirmation(&mut self, device: zbus::ObjectPath<'_>, passkey: u32) -> Result<(), AgentError> {
        self.events.send(Event::PairingDisplay(None));
        match self.ask(device.as_str(), PairingKind::Confirmation { passkey }).await {
            Some(PairingAnswer::Accept) => Ok(()),
            Some(_) => Err(AgentError::Rejected("rejected".into())),
            None => Err(AgentError::Canceled("canceled".into())),
        }
    }

    async fn request_authorization(&mut self, device: zbus::ObjectPath<'_>) -> Result<(), AgentError> {
        match self.ask(device.as_str(), PairingKind::Authorization).await {
            Some(PairingAnswer::Accept) => Ok(()),
            Some(_) => Err(AgentError::Rejected("rejected".into())),
            None => Err(AgentError::Canceled("canceled".into())),
        }
    }

    async fn authorize_service(&mut self, device: zbus::ObjectPath<'_>, uuid: String) -> Result<(), AgentError> {
        let (_, known) = self.device_info(device.as_str()).await;
        if known {
            return Ok(());
        }
        match self.ask(device.as_str(), PairingKind::AuthorizeService { uuid }).await {
            Some(PairingAnswer::Accept) => Ok(()),
            Some(_) => Err(AgentError::Rejected("rejected".into())),
            None => Err(AgentError::Canceled("canceled".into())),
        }
    }

    async fn cancel(&mut self) {
        self.cancel_all();
    }
}

async fn register_agent(
    connection: &Connection,
    app: &str,
    events: EventSink<Event>,
    pending: PairingPending,
) -> zbus::Result<()> {
    let served = connection
        .object_server()
        .at(agent_path(app), PairingAgent { connection: connection.clone(), events, pending, next_id: 0 })
        .await;
    served?;
    let manager = zbus::Proxy::new(connection, SERVICE, "/org/bluez", AGENT_MANAGER_IFACE).await?;
    let path = zbus::ObjectPath::try_from(agent_path(app))?;
    manager.call::<_, _, ()>("RegisterAgent", &(&path, AGENT_CAPABILITY)).await?;
    manager.call::<_, _, ()>("RequestDefaultAgent", &(&path,)).await
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

fn as_percentage(value: &OwnedValue) -> Option<f64> {
    match &**value {
        ZValue::U8(n) => Some(f64::from(*n)),
        ZValue::U32(n) => Some(f64::from(*n)),
        ZValue::I32(n) => Some(f64::from(*n)),
        _ => None,
    }
}

pub fn parse_objects(objects: &Objects) -> BluetoothSnapshot {
    let mut adapters: Vec<(&String, &Properties)> =
        objects.iter().filter_map(|(path, ifaces)| ifaces.get(ADAPTER_IFACE).map(|props| (path, props))).collect();
    adapters.sort_by(|a, b| a.0.cmp(b.0));
    let Some((adapter_path, adapter)) = adapters.first().copied() else {
        return BluetoothSnapshot::default();
    };
    let mut devices: Vec<BluetoothDevice> = objects
        .iter()
        .filter_map(|(path, ifaces)| {
            let props = ifaces.get(DEVICE_IFACE)?;
            if props.get("Adapter").and_then(as_str).as_deref() != Some(adapter_path.as_str()) {
                return None;
            }
            let text = |key: &str| props.get(key).and_then(as_str).unwrap_or_default();
            let flag = |key: &str| props.get(key).and_then(as_bool).unwrap_or(false);
            let battery = ifaces.get(BATTERY_IFACE).and_then(|b| b.get("Percentage")).and_then(as_percentage);
            Some(BluetoothDevice {
                path: path.clone(),
                address: text("Address"),
                name: text("Name"),
                alias: text("Alias"),
                icon: text("Icon"),
                paired: flag("Paired") || flag("Bonded"),
                trusted: flag("Trusted"),
                connected: flag("Connected"),
                battery_available: battery.is_some(),
                battery: battery.map_or(0.0, |p| (p / 100.0).clamp(0.0, 1.0)),
            })
        })
        .collect();
    devices.sort_by(|a, b| a.path.cmp(&b.path));
    BluetoothSnapshot {
        adapter_path: Some(adapter_path.clone()),
        adapter_name: adapter.get("Alias").and_then(as_str).unwrap_or_default(),
        enabled: adapter.get("Powered").and_then(as_bool).unwrap_or(false),
        discovering: adapter.get("Discovering").and_then(as_bool).unwrap_or(false),
        devices,
    }
}

enum Event {
    Snapshot(Box<BluetoothSnapshot>),
    PairingRequested(PairingRequest),
    PairingFinished(u64),
    PairingDisplay(Option<PairingDisplay>),
    AgentRegistered(bool),
    Unavailable,
}

#[derive(Clone)]
pub struct BluetoothState {
    pub ready: Signal<bool>,
    pub available: Signal<bool>,
    pub adapter_name: Signal<String>,
    pub enabled: Signal<bool>,
    pub discovering: Signal<bool>,
    pub devices: Signal<Vec<BluetoothDevice>>,
    pub pairing_requests: Signal<Vec<PairingRequest>>,
    pub pairing_display: Signal<Option<PairingDisplay>>,
    pub agent_registered: Signal<bool>,
}

impl Default for BluetoothState {
    fn default() -> Self {
        Self::new()
    }
}

impl BluetoothState {
    pub fn new() -> Self {
        BluetoothState {
            ready: Signal::new(false),
            available: Signal::new(false),
            adapter_name: Signal::new(String::new()),
            enabled: Signal::new(false),
            discovering: Signal::new(false),
            devices: Signal::new(Vec::new()),
            pairing_requests: Signal::new(Vec::new()),
            pairing_display: Signal::new(None),
            agent_registered: Signal::new(false),
        }
    }

    pub fn apply(&self, snapshot: &BluetoothSnapshot) {
        batch(|| {
            self.available.set(snapshot.adapter_path.is_some());
            self.adapter_name.set(snapshot.adapter_name.clone());
            self.enabled.set(snapshot.enabled);
            self.discovering.set(snapshot.discovering);
            if self.devices.with_untracked(|devices| *devices != snapshot.devices) {
                self.devices.set(snapshot.devices.clone());
            }
            self.ready.set(true);
        });
    }

    pub fn device_by_address(&self, address: &str) -> Option<BluetoothDevice> {
        self.devices.with_untracked(|devices| devices.iter().find(|d| d.address == address).cloned())
    }

    pub fn connected_devices(&self) -> Vec<BluetoothDevice> {
        self.devices.with_untracked(|devices| devices.iter().filter(|d| d.connected).cloned().collect())
    }
}

pub struct Bluetooth {
    pub state: BluetoothState,
    service: ServiceHandle<BluetoothCommand>,
}

impl Bluetooth {
    pub fn start<D: 'static>(handle: &LoopHandle<'static, D>, bus: BusKind, app: &str) -> Result<Bluetooth, IoError> {
        let app = app.to_owned();
        let state = BluetoothState::new();
        let target = state.clone();
        let service = spawn_supervised(
            handle,
            "serp-bluetooth",
            move |event| match event {
                Event::Snapshot(snapshot) => target.apply(&snapshot),
                Event::PairingRequested(request) => target.pairing_requests.update(|list| list.push(request)),
                Event::PairingFinished(id) => target.pairing_requests.update(|list| list.retain(|r| r.id != id)),
                Event::PairingDisplay(display) => target.pairing_display.set(display),
                Event::AgentRegistered(registered) => target.agent_registered.set(registered),
                Event::Unavailable => {
                    target.apply(&BluetoothSnapshot::default());
                    target.agent_registered.set(false);
                }
            },
            move |events, commands| run(bus.clone(), app.clone(), events, commands),
        )?;
        Ok(Bluetooth { state, service })
    }

    pub fn command(&self, command: BluetoothCommand) -> bool {
        self.service.send(command)
    }

    pub fn set_enabled(&self, enabled: bool) -> bool {
        self.command(BluetoothCommand::SetPowered(enabled))
    }

    pub fn set_discovering(&self, discovering: bool) -> bool {
        self.command(BluetoothCommand::SetDiscovering(discovering))
    }

    pub fn connect(&self, path: &str) -> bool {
        self.command(BluetoothCommand::Connect(path.to_owned()))
    }

    pub fn disconnect(&self, path: &str) -> bool {
        self.command(BluetoothCommand::Disconnect(path.to_owned()))
    }

    pub fn pair(&self, path: &str) -> bool {
        self.command(BluetoothCommand::Pair(path.to_owned()))
    }

    pub fn set_trusted(&self, path: &str, trusted: bool) -> bool {
        self.command(BluetoothCommand::SetTrusted(path.to_owned(), trusted))
    }

    pub fn forget(&self, path: &str) -> bool {
        self.command(BluetoothCommand::Forget(path.to_owned()))
    }

    pub fn answer_pairing(&self, id: u64, answer: PairingAnswer) -> bool {
        self.state.pairing_requests.update(|list| list.retain(|r| r.id != id));
        self.command(BluetoothCommand::PairingAnswer { id, answer: Some(answer) })
    }

    pub fn cancel_pairing(&self, id: u64) -> bool {
        self.state.pairing_requests.update(|list| list.retain(|r| r.id != id));
        self.command(BluetoothCommand::PairingAnswer { id, answer: None })
    }
}

async fn read_snapshot(connection: &Connection) -> zbus::Result<BluetoothSnapshot> {
    let manager = zbus::fdo::ObjectManagerProxy::builder(connection).destination(SERVICE).path("/").build().await?;
    let managed = manager.get_managed_objects().await?;
    let objects: Objects = managed
        .into_iter()
        .map(|(path, ifaces)| {
            let ifaces = ifaces.into_iter().map(|(name, props)| (name.to_string(), props)).collect();
            (path.to_string(), ifaces)
        })
        .collect();
    Ok(parse_objects(&objects))
}

async fn set_property(
    connection: &Connection,
    path: &str,
    iface: &str,
    name: &str,
    value: ZValue<'_>,
) -> zbus::Result<()> {
    let properties =
        zbus::fdo::PropertiesProxy::builder(connection).destination(SERVICE).path(path.to_owned()).build().await?;
    properties.set(iface.try_into()?, name, &value).await?;
    Ok(())
}

async fn call_method(connection: &Connection, path: &str, iface: &str, method: &str) -> zbus::Result<()> {
    let proxy = zbus::Proxy::new(connection, SERVICE, path.to_owned(), iface.to_owned()).await?;
    proxy.call::<_, _, ()>(method, &()).await
}

async fn run_command(connection: &Connection, adapter: Option<&str>, command: BluetoothCommand) {
    let result: zbus::Result<()> = async {
        match &command {
            BluetoothCommand::SetPowered(on) => {
                if let Some(adapter) = adapter {
                    set_property(connection, adapter, ADAPTER_IFACE, "Powered", ZValue::from(*on)).await?;
                }
            }
            BluetoothCommand::SetDiscovering(on) => {
                if let Some(adapter) = adapter {
                    let method = if *on { "StartDiscovery" } else { "StopDiscovery" };
                    call_method(connection, adapter, ADAPTER_IFACE, method).await?;
                }
            }
            BluetoothCommand::Connect(path) => call_method(connection, path, DEVICE_IFACE, "Connect").await?,
            BluetoothCommand::Disconnect(path) => call_method(connection, path, DEVICE_IFACE, "Disconnect").await?,
            BluetoothCommand::Pair(path) => {
                let connection = connection.clone();
                let path = path.clone();
                std::thread::spawn(move || {
                    if let Err(e) = async_io::block_on(call_method(&connection, &path, DEVICE_IFACE, "Pair")) {
                        tracing::debug!("bluetooth pair {path} failed: {e}");
                    }
                });
            }
            BluetoothCommand::PairingAnswer { .. } => {}
            BluetoothCommand::SetTrusted(path, trusted) => {
                set_property(connection, path, DEVICE_IFACE, "Trusted", ZValue::from(*trusted)).await?
            }
            BluetoothCommand::Forget(path) => {
                if let Some(adapter) = adapter {
                    let proxy =
                        zbus::Proxy::new(connection, SERVICE, adapter.to_owned(), ADAPTER_IFACE.to_owned()).await?;
                    let device = zbus::ObjectPath::try_from(path.as_str())?;
                    proxy.call::<_, _, ()>("RemoveDevice", &(device,)).await?;
                }
            }
        }
        Ok(())
    }
    .await;
    if let Err(e) = result {
        tracing::debug!("bluetooth command {:?} failed: {e}", command);
    }
}

enum Item {
    Changed,
    OwnerChanged,
    Command(BluetoothCommand),
}

async fn run(bus: BusKind, app: String, events: EventSink<Event>, commands: async_channel::Receiver<BluetoothCommand>) {
    let result: zbus::Result<()> = async {
        let connection = bus.connect_shared().await?;
        let props_rule = zbus::MatchRule::builder()
            .msg_type(zbus::message::Type::Signal)
            .sender(SERVICE)
            .interface("org.freedesktop.DBus.Properties")
            .member("PropertiesChanged")
            .path_namespace("/org/bluez")
            .build()?;
        let added_rule = zbus::MatchRule::builder()
            .msg_type(zbus::message::Type::Signal)
            .sender(SERVICE)
            .interface("org.freedesktop.DBus.ObjectManager")
            .member("InterfacesAdded")
            .build()?;
        let removed_rule = zbus::MatchRule::builder()
            .msg_type(zbus::message::Type::Signal)
            .sender(SERVICE)
            .interface("org.freedesktop.DBus.ObjectManager")
            .member("InterfacesRemoved")
            .build()?;
        let props =
            zbus::MessageStream::for_match_rule(props_rule, &connection, None).await?.map(|m| m.map(|_| Item::Changed));
        let added =
            zbus::MessageStream::for_match_rule(added_rule, &connection, None).await?.map(|m| m.map(|_| Item::Changed));
        let removed = zbus::MessageStream::for_match_rule(removed_rule, &connection, None)
            .await?
            .map(|m| m.map(|_| Item::Changed));
        let owner_rule = zbus::MatchRule::builder()
            .msg_type(zbus::message::Type::Signal)
            .sender("org.freedesktop.DBus")
            .interface("org.freedesktop.DBus")
            .member("NameOwnerChanged")
            .arg(0, SERVICE)
            .build()?;
        let owner = zbus::MessageStream::for_match_rule(owner_rule, &connection, None)
            .await?
            .map(|m| m.map(|_| Item::OwnerChanged));
        let cmds = commands.map(|c| Ok(Item::Command(c)));
        let mut stream = std::pin::pin!(props.or(added).or(removed).or(owner).or(cmds));

        let pending: PairingPending = Arc::default();
        let mut registered = false;
        let mut current = read_snapshot(&connection).await?;
        events.send(Event::Snapshot(Box::new(current.clone())));
        loop {
            if !registered && current.adapter_path.is_some() {
                match register_agent(&connection, &app, events.clone(), pending.clone()).await {
                    Ok(()) => {
                        registered = true;
                        events.send(Event::AgentRegistered(true));
                    }
                    Err(e) => tracing::debug!("bluetooth agent registration failed: {e}"),
                }
            }
            let Some(item) = stream.next().await else { break };
            let Ok(item) = item else { continue };
            if let Item::Command(command) = item {
                if let BluetoothCommand::PairingAnswer { id, answer } = command {
                    let sender = pending.lock().expect("pairing lock").remove(&id);
                    if let Some(sender) = sender {
                        let _ = sender.try_send(answer);
                    }
                    continue;
                }
                run_command(&connection, current.adapter_path.as_deref(), command).await;
                continue;
            }
            if matches!(item, Item::OwnerChanged) && registered {
                registered = false;
                events.send(Event::AgentRegistered(false));
            }
            match read_snapshot(&connection).await {
                Ok(next) if next != current => {
                    current = next;
                    events.send(Event::Snapshot(Box::new(current.clone())));
                }
                Ok(_) => {}
                Err(e) => tracing::debug!("bluetooth refresh failed: {e}"),
            }
        }
        Ok(())
    }
    .await;
    if let Err(e) = result {
        tracing::info!("bluetooth unavailable: {e}");
        events.send(Event::Unavailable);
    }
}

#[cfg(test)]
mod tests {
    use std::sync::mpsc;
    use std::time::{Duration, Instant};

    use calloop::EventLoop;
    use zbus::interface;

    use super::*;
    use crate::platform::dbus::testbus::TestBus;

    fn props(entries: &[(&str, ZValue<'static>)]) -> Properties {
        entries.iter().map(|(k, v)| ((*k).to_owned(), OwnedValue::try_from(v.clone()).unwrap())).collect()
    }

    fn sample_objects() -> Objects {
        let mut objects = Objects::new();
        objects.insert(
            "/org/bluez/hci0".into(),
            HashMap::from([(
                ADAPTER_IFACE.to_owned(),
                props(&[
                    ("Alias", ZValue::from("laptop")),
                    ("Powered", ZValue::from(true)),
                    ("Discovering", ZValue::from(false)),
                ]),
            )]),
        );
        objects.insert(
            "/org/bluez/hci0/dev_AA".into(),
            HashMap::from([
                (
                    DEVICE_IFACE.to_owned(),
                    props(&[
                        ("Adapter", ZValue::from(zbus::ObjectPath::from_static_str_unchecked("/org/bluez/hci0"))),
                        ("Address", ZValue::from("AA:BB")),
                        ("Name", ZValue::from("Buds")),
                        ("Alias", ZValue::from("My buds")),
                        ("Icon", ZValue::from("audio-headset")),
                        ("Paired", ZValue::from(true)),
                        ("Connected", ZValue::from(true)),
                    ]),
                ),
                (BATTERY_IFACE.to_owned(), props(&[("Percentage", ZValue::from(80u8))])),
            ]),
        );
        objects.insert(
            "/org/bluez/hci1/dev_ZZ".into(),
            HashMap::from([(
                DEVICE_IFACE.to_owned(),
                props(&[
                    ("Adapter", ZValue::from(zbus::ObjectPath::from_static_str_unchecked("/org/bluez/hci1"))),
                    ("Address", ZValue::from("ZZ")),
                ]),
            )]),
        );
        objects
    }

    #[test]
    fn objects_parse_into_adapter_and_devices() {
        let snapshot = parse_objects(&sample_objects());
        assert_eq!(snapshot.adapter_path.as_deref(), Some("/org/bluez/hci0"));
        assert_eq!(snapshot.adapter_name, "laptop");
        assert!(snapshot.enabled);
        assert!(!snapshot.discovering);
        assert_eq!(snapshot.devices.len(), 1);
        let device = &snapshot.devices[0];
        assert_eq!(device.address, "AA:BB");
        assert_eq!(device.display_name(), "Buds");
        assert!(device.paired && device.connected && !device.trusted);
        assert!(device.battery_available);
        assert_eq!(device.battery, 0.8);
    }

    #[test]
    fn no_adapter_means_unavailable() {
        let snapshot = parse_objects(&Objects::new());
        assert!(snapshot.adapter_path.is_none());
        assert!(snapshot.devices.is_empty());
    }

    #[test]
    fn display_name_falls_back_to_alias_then_address() {
        let mut device = BluetoothDevice { address: "AA".into(), alias: "Alias".into(), ..Default::default() };
        assert_eq!(device.display_name(), "Alias");
        device.alias.clear();
        assert_eq!(device.display_name(), "AA");
    }

    struct FakeAdapter {
        powered: bool,
        discovering: bool,
        events: mpsc::Sender<String>,
    }

    #[interface(name = "org.bluez.Adapter1")]
    impl FakeAdapter {
        #[zbus(property)]
        fn powered(&self) -> bool {
            self.powered
        }
        #[zbus(property)]
        fn set_powered(&mut self, value: bool) {
            self.powered = value;
        }
        #[zbus(property)]
        fn discovering(&self) -> bool {
            self.discovering
        }
        #[zbus(property)]
        fn alias(&self) -> String {
            "fake".into()
        }
        async fn start_discovery(&mut self, #[zbus(signal_emitter)] emitter: zbus::object_server::SignalEmitter<'_>) {
            self.discovering = true;
            let _ = self.events.send("start".into());
            let _ = self.discovering_changed(&emitter).await;
        }
        async fn stop_discovery(&mut self, #[zbus(signal_emitter)] emitter: zbus::object_server::SignalEmitter<'_>) {
            self.discovering = false;
            let _ = self.events.send("stop".into());
            let _ = self.discovering_changed(&emitter).await;
        }
    }

    struct FakeDevice {
        connected: bool,
    }

    #[interface(name = "org.bluez.Device1")]
    impl FakeDevice {
        #[zbus(property)]
        fn adapter(&self) -> zbus::OwnedObjectPath {
            zbus::OwnedObjectPath::try_from("/org/bluez/hci0").unwrap()
        }
        #[zbus(property)]
        fn address(&self) -> String {
            "AA:BB".into()
        }
        #[zbus(property)]
        fn name(&self) -> String {
            "Buds".into()
        }
        #[zbus(property)]
        fn connected(&self) -> bool {
            self.connected
        }
        async fn connect(&mut self, #[zbus(signal_emitter)] emitter: zbus::object_server::SignalEmitter<'_>) {
            self.connected = true;
            let _ = self.connected_changed(&emitter).await;
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
    fn missing_service_leaves_bluetooth_unavailable() {
        let bus = TestBus::start();
        let mut event_loop: EventLoop<'static, ()> = EventLoop::try_new().unwrap();
        let bluetooth = Bluetooth::start(&event_loop.handle(), bus.kind(), "demo").unwrap();
        assert!(pump_until(&mut event_loop, || bluetooth.state.ready.get_untracked()));
        assert!(!bluetooth.state.available.get_untracked());
        assert!(bluetooth.state.devices.get_untracked().is_empty());
    }

    #[test]
    fn commands_reach_the_service_and_changes_flow_back() {
        let bus = TestBus::start();
        let (event_tx, event_rx) = mpsc::channel();
        let (ready_tx, ready_rx) = mpsc::channel();
        let address = bus.address.clone();
        std::thread::spawn(move || {
            async_io::block_on(async move {
                let _connection = zbus::connection::Builder::address(address.as_str())
                    .name(SERVICE)
                    .serve_at("/", zbus::fdo::ObjectManager)
                    .serve_at("/org/bluez/hci0", FakeAdapter { powered: true, discovering: false, events: event_tx })
                    .serve_at("/org/bluez/hci0/dev_AA", FakeDevice { connected: false })
                    .build()
                    .await
                    .unwrap();
                ready_tx.send(()).unwrap();
                std::future::pending::<()>().await;
            });
        });
        ready_rx.recv_timeout(Duration::from_secs(10)).unwrap();

        let mut event_loop: EventLoop<'static, ()> = EventLoop::try_new().unwrap();
        let bluetooth = Bluetooth::start(&event_loop.handle(), bus.kind(), "demo").unwrap();
        let state = bluetooth.state.clone();
        assert!(pump_until(&mut event_loop, || state.ready.get_untracked()));
        assert!(state.available.get_untracked());
        assert!(state.enabled.get_untracked());
        assert_eq!(state.adapter_name.get_untracked(), "fake");
        let device = state.device_by_address("AA:BB").unwrap();
        assert_eq!(device.name, "Buds");
        assert!(!device.connected);

        assert!(bluetooth.set_discovering(true));
        assert_eq!(event_rx.recv_timeout(Duration::from_secs(10)).unwrap(), "start");
        assert!(pump_until(&mut event_loop, || state.discovering.get_untracked()));

        assert!(bluetooth.connect(&device.path));
        assert!(pump_until(&mut event_loop, || state.connected_devices().len() == 1));

        assert!(bluetooth.set_enabled(false));
        assert!(pump_until(&mut event_loop, || !state.enabled.get_untracked()));
    }

    struct FakeAgentManager {
        events: mpsc::Sender<String>,
    }

    #[interface(name = "org.bluez.AgentManager1")]
    impl FakeAgentManager {
        fn register_agent(
            &self,
            path: zbus::ObjectPath<'_>,
            capability: String,
            #[zbus(header)] header: zbus::message::Header<'_>,
        ) {
            let sender = header.sender().map(|s| s.to_string()).unwrap_or_default();
            let _ = self.events.send(format!("{sender} {path} {capability}"));
        }

        fn request_default_agent(&self, path: zbus::ObjectPath<'_>) {
            let _ = self.events.send(format!("default {path}"));
        }
    }

    struct FakePairDevice {
        alias: &'static str,
        paired: bool,
    }

    #[interface(name = "org.bluez.Device1")]
    impl FakePairDevice {
        #[zbus(property)]
        fn adapter(&self) -> zbus::OwnedObjectPath {
            zbus::OwnedObjectPath::try_from("/org/bluez/hci0").unwrap()
        }
        #[zbus(property)]
        fn address(&self) -> String {
            self.alias.to_uppercase()
        }
        #[zbus(property)]
        fn alias(&self) -> String {
            self.alias.into()
        }
        #[zbus(property)]
        fn paired(&self) -> bool {
            self.paired
        }
    }

    #[test]
    fn the_pairing_agent_registers_and_relays_requests() {
        let bus = TestBus::start();
        let (event_tx, event_rx) = mpsc::channel();
        let (agent_tx, agent_rx) = mpsc::channel();
        let (ready_tx, ready_rx) = mpsc::channel();
        let address = bus.address.clone();
        std::thread::spawn(move || {
            async_io::block_on(async move {
                let _connection = zbus::connection::Builder::address(address.as_str())
                    .name(SERVICE)
                    .serve_at("/", zbus::fdo::ObjectManager)
                    .serve_at("/org/bluez", FakeAgentManager { events: agent_tx })
                    .serve_at("/org/bluez/hci0", FakeAdapter { powered: true, discovering: false, events: event_tx })
                    .serve_at("/org/bluez/hci0/dev_AA", FakePairDevice { alias: "Buds", paired: false })
                    .serve_at("/org/bluez/hci0/dev_BB", FakePairDevice { alias: "Known", paired: true })
                    .build()
                    .await
                    .unwrap();
                ready_tx.send(()).unwrap();
                std::future::pending::<()>().await;
            });
        });
        ready_rx.recv_timeout(Duration::from_secs(10)).unwrap();
        drop(event_rx);

        let mut event_loop: EventLoop<'static, ()> = EventLoop::try_new().unwrap();
        let bluetooth = Bluetooth::start(&event_loop.handle(), bus.kind(), "demo").unwrap();
        let state = bluetooth.state.clone();
        assert!(pump_until(&mut event_loop, || state.agent_registered.get_untracked()));
        let registration = agent_rx.recv_timeout(Duration::from_secs(10)).unwrap();
        let (owner, rest) = registration.split_once(' ').unwrap();
        assert_eq!(rest, "/org/demo/BluezAgent KeyboardDisplay");
        assert_eq!(agent_rx.recv_timeout(Duration::from_secs(10)).unwrap(), "default /org/demo/BluezAgent");

        let (result_tx, result_rx) = mpsc::channel::<Result<String, String>>();
        let client_bus = bus.kind();
        let owner = owner.to_owned();
        std::thread::spawn(move || {
            async_io::block_on(async move {
                let client = client_bus.connect().await.unwrap();
                let agent = zbus::Proxy::new(&client, owner, agent_path("demo"), "org.bluez.Agent1").await.unwrap();
                let buds = zbus::ObjectPath::try_from("/org/bluez/hci0/dev_AA").unwrap();
                let known = zbus::ObjectPath::try_from("/org/bluez/hci0/dev_BB").unwrap();
                let name = |e: zbus::Error| match e {
                    zbus::Error::MethodError(name, ..) => name.to_string(),
                    other => other.to_string(),
                };
                let pin: zbus::Result<String> = agent.call("RequestPinCode", &(&buds,)).await;
                result_tx.send(pin.map_err(name)).unwrap();
                let confirm: zbus::Result<()> = agent.call("RequestConfirmation", &(&buds, 123456u32)).await;
                result_tx.send(confirm.map(|_| "ok".to_owned()).map_err(name)).unwrap();
                let _: zbus::Result<()> = agent.call("DisplayPasskey", &(&buds, 42u32, 1u16)).await;
                result_tx.send(Ok("displayed".into())).unwrap();
                let service: zbus::Result<()> = agent.call("AuthorizeService", &(&known, "0000110b".to_owned())).await;
                result_tx.send(service.map(|_| "authorized".to_owned()).map_err(name)).unwrap();
                let cancelled: zbus::Result<String> = agent.call("RequestPinCode", &(&buds,)).await;
                result_tx.send(cancelled.map_err(name)).unwrap();
            });
        });

        assert!(pump_until(&mut event_loop, || !state.pairing_requests.get_untracked().is_empty()));
        let request = state.pairing_requests.get_untracked()[0].clone();
        assert_eq!((request.kind.clone(), request.device_name.as_str()), (PairingKind::PinCode, "Buds"));
        assert!(bluetooth.answer_pairing(request.id, PairingAnswer::Pin("0000".into())));
        assert_eq!(result_rx.recv_timeout(Duration::from_secs(10)).unwrap(), Ok("0000".to_owned()));

        assert!(pump_until(&mut event_loop, || !state.pairing_requests.get_untracked().is_empty()));
        let request = state.pairing_requests.get_untracked()[0].clone();
        assert_eq!(request.kind, PairingKind::Confirmation { passkey: 123456 });
        assert!(bluetooth.answer_pairing(request.id, PairingAnswer::Reject));
        assert_eq!(result_rx.recv_timeout(Duration::from_secs(10)).unwrap(), Err("org.bluez.Error.Rejected".into()));

        assert_eq!(result_rx.recv_timeout(Duration::from_secs(10)).unwrap(), Ok("displayed".to_owned()));
        assert!(pump_until(&mut event_loop, || state.pairing_display.get_untracked().is_some()));
        let display = state.pairing_display.get_untracked().unwrap();
        assert_eq!((display.code.as_str(), display.entered, display.device_name.as_str()), ("000042", 1, "Buds"));

        assert_eq!(result_rx.recv_timeout(Duration::from_secs(10)).unwrap(), Ok("authorized".to_owned()));
        assert!(state.pairing_requests.get_untracked().is_empty());

        assert!(pump_until(&mut event_loop, || !state.pairing_requests.get_untracked().is_empty()));
        let request = state.pairing_requests.get_untracked()[0].clone();
        assert!(bluetooth.cancel_pairing(request.id));
        assert_eq!(result_rx.recv_timeout(Duration::from_secs(10)).unwrap(), Err("org.bluez.Error.Canceled".into()));
    }
}
