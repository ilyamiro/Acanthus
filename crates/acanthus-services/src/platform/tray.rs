use std::collections::HashMap;
use std::path::Path;
use std::sync::{Arc, Mutex};

use acanthus_core::{Signal, batch};
use acanthus_io::IoError;
use calloop::LoopHandle;
use futures_lite::StreamExt;
use zbus::object_server::SignalEmitter;
use zbus::{Connection, OwnedVariant as OwnedValue, Variant as ZValue, interface};

use crate::platform::bridge::{EventSink, ServiceHandle, spawn_supervised};
use crate::platform::dbus::BusKind;

const WATCHER_NAME: &str = "org.kde.StatusNotifierWatcher";
const WATCHER_PATH: &str = "/StatusNotifierWatcher";
const ITEM_IFACE: &str = "org.kde.StatusNotifierItem";
const MENU_IFACE: &str = "com.canonical.dbusmenu";
const DEFAULT_ITEM_PATH: &str = "/StatusNotifierItem";

type Pixmaps = Vec<(i32, i32, Vec<u8>)>;
type Layout = (i32, HashMap<String, OwnedValue>, Vec<OwnedValue>);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TrayStatus {
    Passive,
    #[default]
    Active,
    NeedsAttention,
}

impl TrayStatus {
    fn parse(text: &str) -> TrayStatus {
        match text {
            "Passive" => TrayStatus::Passive,
            "NeedsAttention" => TrayStatus::NeedsAttention,
            _ => TrayStatus::Active,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrayPixmap {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct TrayItem {
    pub key: String,
    pub id: String,
    pub title: String,
    pub category: String,
    pub status: TrayStatus,
    pub icon: String,
    pub attention_icon: String,
    pub overlay_icon: String,
    pub pixmap: Option<TrayPixmap>,
    pub tooltip_icon: String,
    pub tooltip_pixmap: Option<TrayPixmap>,
    pub tooltip_title: String,
    pub tooltip_text: String,
    pub menu_path: Option<String>,
    pub is_menu_only: bool,
}

impl TrayItem {
    pub fn has_menu(&self) -> bool {
        self.menu_path.is_some()
    }

    pub fn display_name(&self) -> &str {
        [&self.title, &self.tooltip_title, &self.id].into_iter().find(|s| !s.is_empty()).map_or("", |s| s.as_str())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum MenuToggle {
    #[default]
    None,
    Check,
    Radio,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct MenuEntry {
    pub id: i32,
    pub text: String,
    pub icon: String,
    pub enabled: bool,
    pub separator: bool,
    pub toggle: MenuToggle,
    pub checked: bool,
    pub children: Vec<MenuEntry>,
}

impl MenuEntry {
    pub fn has_children(&self) -> bool {
        !self.children.is_empty()
    }
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct TrayMenu {
    pub key: String,
    pub entries: Vec<MenuEntry>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum TrayCommand {
    Activate(String, i32, i32),
    SecondaryActivate(String, i32, i32),
    ContextMenu(String, i32, i32),
    Scroll(String, i32, bool),
    LoadMenu(String),
    CloseMenu,
    MenuClick(String, i32),
}

pub fn item_address(service: &str, sender: &str) -> (String, String) {
    if service.starts_with('/') {
        (sender.to_owned(), service.to_owned())
    } else if let Some(index) = service.find('/') {
        (service[..index].to_owned(), service[index..].to_owned())
    } else {
        (service.to_owned(), DEFAULT_ITEM_PATH.to_owned())
    }
}

pub fn argb_to_rgba(argb: &[u8]) -> Vec<u8> {
    argb.as_chunks::<4>().0.iter().flat_map(|p| [p[1], p[2], p[3], p[0]]).collect()
}

pub fn best_pixmap(pixmaps: &Pixmaps) -> Option<TrayPixmap> {
    pixmaps
        .iter()
        .filter(|(w, h, data)| *w > 0 && *h > 0 && data.len() == (*w as usize) * (*h as usize) * 4)
        .max_by_key(|(w, h, _)| i64::from(*w) * i64::from(*h))
        .map(|(w, h, data)| TrayPixmap { width: *w as u32, height: *h as u32, rgba: argb_to_rgba(data) })
}

pub fn strip_mnemonic(label: &str) -> String {
    let mut out = String::with_capacity(label.len());
    let mut chars = label.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '_' {
            if chars.peek() == Some(&'_') {
                out.push('_');
                chars.next();
            }
        } else {
            out.push(c);
        }
    }
    out
}

type Props = HashMap<String, OwnedValue>;

fn text(props: &Props, key: &str) -> String {
    match props.get(key).map(|v| &**v) {
        Some(ZValue::Str(s)) => s.to_string(),
        Some(ZValue::ObjectPath(p)) => p.to_string(),
        _ => String::new(),
    }
}

fn flag(props: &Props, key: &str, default: bool) -> bool {
    match props.get(key).map(|v| &**v) {
        Some(ZValue::Bool(b)) => *b,
        _ => default,
    }
}

fn pixmaps(props: &Props, key: &str) -> Pixmaps {
    props.get(key).and_then(|v| v.try_clone().ok()).and_then(|v| Pixmaps::try_from(v).ok()).unwrap_or_default()
}

fn resolve_icon(name: &str, theme_path: &str) -> String {
    if name.is_empty() || name.starts_with('/') || theme_path.is_empty() {
        return name.to_owned();
    }
    for extension in ["png", "svg", "xpm"] {
        let candidate = Path::new(theme_path).join(format!("{name}.{extension}"));
        if candidate.is_file() {
            return candidate.to_string_lossy().into_owned();
        }
    }
    name.to_owned()
}

pub fn build_item(key: &str, props: &Props) -> TrayItem {
    let theme_path = text(props, "IconThemePath");
    let tooltip = props
        .get("ToolTip")
        .and_then(|v| v.try_clone().ok())
        .and_then(|v| <(String, Pixmaps, String, String)>::try_from(v).ok())
        .unwrap_or_default();
    let mut icon_pixmaps = pixmaps(props, "IconPixmap");
    if icon_pixmaps.is_empty() {
        icon_pixmaps = pixmaps(props, "AttentionIconPixmap");
    }
    let menu = text(props, "Menu");
    TrayItem {
        key: key.to_owned(),
        id: text(props, "Id"),
        title: text(props, "Title"),
        category: text(props, "Category"),
        status: TrayStatus::parse(&text(props, "Status")),
        icon: resolve_icon(&text(props, "IconName"), &theme_path),
        attention_icon: resolve_icon(&text(props, "AttentionIconName"), &theme_path),
        overlay_icon: resolve_icon(&text(props, "OverlayIconName"), &theme_path),
        pixmap: best_pixmap(&icon_pixmaps),
        tooltip_icon: resolve_icon(&tooltip.0, &theme_path),
        tooltip_pixmap: best_pixmap(&tooltip.1),
        tooltip_title: tooltip.2,
        tooltip_text: tooltip.3,
        menu_path: (!menu.is_empty() && menu != "/").then_some(menu),
        is_menu_only: flag(props, "ItemIsMenu", false),
    }
}

fn parse_entry(id: i32, props: &Props, children: &[OwnedValue]) -> MenuEntry {
    let toggle = match text(props, "toggle-type").as_str() {
        "checkmark" => MenuToggle::Check,
        "radio" => MenuToggle::Radio,
        _ => MenuToggle::None,
    };
    let checked = matches!(props.get("toggle-state").map(|v| &**v), Some(ZValue::I32(1)));
    MenuEntry {
        id,
        text: strip_mnemonic(&text(props, "label")),
        icon: text(props, "icon-name"),
        enabled: flag(props, "enabled", true),
        separator: text(props, "type") == "separator",
        toggle,
        checked,
        children: parse_children(children),
    }
}

fn parse_children(children: &[OwnedValue]) -> Vec<MenuEntry> {
    children
        .iter()
        .filter_map(|child| {
            let child = child.try_clone().ok()?;
            let (id, props, grandchildren) = Layout::try_from(child).ok()?;
            flag(&props, "visible", true).then(|| parse_entry(id, &props, &grandchildren))
        })
        .collect()
}

pub fn parse_layout(layout: &Layout) -> Vec<MenuEntry> {
    parse_children(&layout.2)
}

#[derive(Debug, Clone)]
struct Tracked {
    key: String,
    bus: String,
    path: String,
    owner: String,
    item: TrayItem,
}

enum Event {
    Watcher(bool),
    Items(Vec<TrayItem>),
    Menu(TrayMenu),
    Unavailable,
}

enum Input {
    Register(String),
    Unregister(String),
    ItemSignal(String),
    MenuSignal(String),
    OwnerGone(String),
    Command(TrayCommand),
    Ignore,
}

#[derive(Clone)]
pub struct TrayState {
    pub ready: Signal<bool>,
    pub available: Signal<bool>,
    pub items: Signal<Vec<TrayItem>>,
    pub menu: Signal<Option<TrayMenu>>,
}

impl TrayState {
    fn new() -> Self {
        TrayState {
            ready: Signal::new(false),
            available: Signal::new(false),
            items: Signal::new(Vec::new()),
            menu: Signal::new(None),
        }
    }

    pub fn item(&self, key: &str) -> Option<TrayItem> {
        self.items.with_untracked(|items| items.iter().find(|i| i.key == key).cloned())
    }

    pub fn item_by_id(&self, id: &str) -> Option<TrayItem> {
        self.items.with_untracked(|items| items.iter().find(|i| i.id == id).cloned())
    }
}

pub struct Tray {
    pub state: TrayState,
    service: ServiceHandle<TrayCommand>,
}

impl Tray {
    pub fn start<D: 'static>(handle: &LoopHandle<'static, D>, bus: BusKind) -> Result<Tray, IoError> {
        let state = TrayState::new();
        let target = state.clone();
        let service = spawn_supervised(
            handle,
            "serp-tray",
            move |event| match event {
                Event::Watcher(available) => batch(|| {
                    target.available.set(available);
                    target.ready.set(true);
                }),
                Event::Items(items) => batch(|| {
                    target.items.set(items);
                    target.ready.set(true);
                }),
                Event::Menu(menu) => {
                    if target.menu.with_untracked(|current| current.as_ref() != Some(&menu)) {
                        target.menu.set(Some(menu));
                    }
                }
                Event::Unavailable => batch(|| {
                    target.available.set(false);
                    target.items.set(Vec::new());
                    target.ready.set(true);
                }),
            },
            move |events, commands| run(bus.clone(), events, commands),
        )?;
        Ok(Tray { state, service })
    }

    pub fn command(&self, command: TrayCommand) -> bool {
        self.service.send(command)
    }

    pub fn activate(&self, key: &str, x: i32, y: i32) -> bool {
        self.command(TrayCommand::Activate(key.to_owned(), x, y))
    }

    pub fn secondary_activate(&self, key: &str, x: i32, y: i32) -> bool {
        self.command(TrayCommand::SecondaryActivate(key.to_owned(), x, y))
    }

    pub fn context_menu(&self, key: &str, x: i32, y: i32) -> bool {
        self.command(TrayCommand::ContextMenu(key.to_owned(), x, y))
    }

    pub fn scroll(&self, key: &str, delta: i32, horizontal: bool) -> bool {
        self.command(TrayCommand::Scroll(key.to_owned(), delta, horizontal))
    }

    pub fn load_menu(&self, key: &str) -> bool {
        self.command(TrayCommand::LoadMenu(key.to_owned()))
    }

    pub fn click_menu_entry(&self, key: &str, id: i32) -> bool {
        self.command(TrayCommand::MenuClick(key.to_owned(), id))
    }

    pub fn close_menu(&self) {
        self.state.menu.set(None);
        self.command(TrayCommand::CloseMenu);
    }
}

struct Watcher {
    registered: Arc<Mutex<Vec<String>>>,
    inputs: async_channel::Sender<Input>,
}

#[interface(name = "org.kde.StatusNotifierWatcher")]
impl Watcher {
    async fn register_status_notifier_item(
        &self,
        service: &str,
        #[zbus(header)] header: zbus::message::Header<'_>,
        #[zbus(signal_emitter)] emitter: SignalEmitter<'_>,
    ) -> zbus::fdo::Result<()> {
        let sender = header.sender().map(|s| s.to_string()).unwrap_or_default();
        let (bus, path) = item_address(service, &sender);
        let key = format!("{bus}{path}");
        let added = self.registered.lock().is_ok_and(|mut items| {
            let fresh = !items.contains(&key);
            if fresh {
                items.push(key.clone());
            }
            fresh
        });
        if added {
            let _ = self.inputs.send(Input::Register(key.clone())).await;
            let _ = Self::status_notifier_item_registered(&emitter, &key).await;
        }
        Ok(())
    }

    async fn register_status_notifier_host(
        &self,
        _service: &str,
        #[zbus(signal_emitter)] emitter: SignalEmitter<'_>,
    ) -> zbus::fdo::Result<()> {
        let _ = Self::status_notifier_host_registered(&emitter).await;
        Ok(())
    }

    #[zbus(property)]
    fn registered_status_notifier_items(&self) -> Vec<String> {
        self.registered.lock().map(|items| items.clone()).unwrap_or_default()
    }

    #[zbus(property)]
    fn is_status_notifier_host_registered(&self) -> bool {
        true
    }

    #[zbus(property)]
    fn protocol_version(&self) -> i32 {
        0
    }

    #[zbus(signal)]
    async fn status_notifier_item_registered(emitter: &SignalEmitter<'_>, service: &str) -> zbus::Result<()>;

    #[zbus(signal)]
    async fn status_notifier_item_unregistered(emitter: &SignalEmitter<'_>, service: &str) -> zbus::Result<()>;

    #[zbus(signal)]
    async fn status_notifier_host_registered(emitter: &SignalEmitter<'_>) -> zbus::Result<()>;
}

async fn owner_of(connection: &Connection, bus: &str) -> Option<String> {
    if bus.starts_with(':') {
        return Some(bus.to_owned());
    }
    let proxy = zbus::fdo::DBusProxy::new(connection).await.ok()?;
    proxy.get_name_owner(bus.to_owned().try_into().ok()?).await.ok().map(|n| n.to_string())
}

async fn fetch_item(connection: &Connection, bus: &str, path: &str, key: &str) -> zbus::Result<TrayItem> {
    let proxy = zbus::fdo::PropertiesProxy::builder(connection)
        .destination(bus.to_owned())
        .path(path.to_owned())
        .build()
        .await?;
    let props = proxy.get_all(ITEM_IFACE.try_into()?).await?;
    Ok(build_item(key, &props))
}

async fn fetch_menu(connection: &Connection, tracked: &Tracked, about_to_show: bool) -> zbus::Result<Vec<MenuEntry>> {
    let menu_path = tracked.item.menu_path.clone().ok_or_else(|| zbus::Error::Failure("item has no menu".into()))?;
    let proxy = zbus::Proxy::new(connection, tracked.bus.clone(), menu_path, MENU_IFACE).await?;
    if about_to_show {
        let _ = proxy.call::<_, _, bool>("AboutToShow", &(0i32,)).await;
    }
    let (_revision, layout): (u32, Layout) = proxy.call("GetLayout", &(0i32, -1i32, Vec::<String>::new())).await?;
    Ok(parse_layout(&layout))
}

async fn run_command(connection: &Connection, items: &[Tracked], command: TrayCommand, events: &EventSink<Event>) {
    let key = match &command {
        TrayCommand::CloseMenu => return,
        TrayCommand::Activate(k, ..)
        | TrayCommand::SecondaryActivate(k, ..)
        | TrayCommand::ContextMenu(k, ..)
        | TrayCommand::Scroll(k, ..)
        | TrayCommand::LoadMenu(k)
        | TrayCommand::MenuClick(k, _) => k.clone(),
    };
    let Some(tracked) = items.iter().find(|t| t.key == key) else { return };
    let result: zbus::Result<()> = async {
        match &command {
            TrayCommand::Activate(_, x, y) => call_item(connection, tracked, "Activate", &(*x, *y)).await?,
            TrayCommand::SecondaryActivate(_, x, y) => {
                call_item(connection, tracked, "SecondaryActivate", &(*x, *y)).await?
            }
            TrayCommand::ContextMenu(_, x, y) => call_item(connection, tracked, "ContextMenu", &(*x, *y)).await?,
            TrayCommand::Scroll(_, delta, horizontal) => {
                let orientation = if *horizontal { "horizontal" } else { "vertical" };
                call_item(connection, tracked, "Scroll", &(*delta, orientation)).await?
            }
            TrayCommand::CloseMenu => {}
            TrayCommand::LoadMenu(_) => {
                let entries = fetch_menu(connection, tracked, true).await?;
                events.send(Event::Menu(TrayMenu { key: tracked.key.clone(), entries }));
            }
            TrayCommand::MenuClick(_, id) => {
                let menu_path = tracked.item.menu_path.clone().unwrap_or_default();
                let proxy = zbus::Proxy::new(connection, tracked.bus.clone(), menu_path, MENU_IFACE).await?;
                proxy.call::<_, _, ()>("Event", &(*id, "clicked", ZValue::from(0i32), 0u32)).await?;
            }
        }
        Ok(())
    }
    .await;
    if let Err(e) = result {
        tracing::debug!("tray command {command:?} failed: {e}");
    }
}

async fn call_item<B>(connection: &Connection, tracked: &Tracked, method: &str, body: &B) -> zbus::Result<()>
where
    B: serde::Serialize + zbus::zvariant::DynamicType,
{
    let proxy = zbus::Proxy::new(connection, tracked.bus.clone(), tracked.path.clone(), ITEM_IFACE).await?;
    proxy.call::<_, _, ()>(method, body).await
}

async fn track(connection: &Connection, raw: &str) -> Option<Tracked> {
    let (bus, path) = item_address(raw, "");
    if bus.is_empty() {
        return None;
    }
    let owner = owner_of(connection, &bus).await?;
    let key = format!("{bus}{path}");
    match fetch_item(connection, &bus, &path, &key).await {
        Ok(item) => Some(Tracked { key, bus, path, owner, item }),
        Err(e) => {
            tracing::debug!("tray item {raw} unreadable: {e}");
            None
        }
    }
}

fn publish(events: &EventSink<Event>, items: &[Tracked], last: &mut Vec<TrayItem>) {
    let next: Vec<TrayItem> = items.iter().map(|t| t.item.clone()).collect();
    if next != *last {
        last.clone_from(&next);
        events.send(Event::Items(next));
    }
}

async fn run(bus: BusKind, events: EventSink<Event>, commands: async_channel::Receiver<TrayCommand>) {
    let result: zbus::Result<()> = async {
        let connection = bus.connect().await?;
        let (input_tx, input_rx) = async_channel::unbounded::<Input>();
        let registered: Arc<Mutex<Vec<String>>> = Arc::default();
        connection
            .object_server()
            .at(WATCHER_PATH, Watcher { registered: registered.clone(), inputs: input_tx.clone() })
            .await?;
        let owns_watcher =
            connection.request_name_with_flags(WATCHER_NAME, zbus::fdo::RequestNameFlags::DO_NOT_QUEUE).await.is_ok();
        let host_name = format!("org.kde.StatusNotifierHost-{}", std::process::id());
        let _ = connection.request_name(host_name.as_str()).await;

        let item_rule =
            zbus::MatchRule::builder().msg_type(zbus::message::Type::Signal).interface(ITEM_IFACE).build()?;
        let menu_rule =
            zbus::MatchRule::builder().msg_type(zbus::message::Type::Signal).interface(MENU_IFACE).build()?;
        let owner_rule = zbus::MatchRule::builder()
            .msg_type(zbus::message::Type::Signal)
            .sender("org.freedesktop.DBus")
            .interface("org.freedesktop.DBus")
            .member("NameOwnerChanged")
            .build()?;
        let item_signals = zbus::MessageStream::for_match_rule(item_rule, &connection, None).await?.map(|m| {
            m.ok().and_then(|m| m.header().sender().map(|s| s.to_string())).map_or(Input::Ignore, Input::ItemSignal)
        });
        let menu_signals = zbus::MessageStream::for_match_rule(menu_rule, &connection, None).await?.map(|m| {
            m.ok().and_then(|m| m.header().sender().map(|s| s.to_string())).map_or(Input::Ignore, Input::MenuSignal)
        });
        let owner_changes = zbus::MessageStream::for_match_rule(owner_rule, &connection, None).await?.map(|m| {
            m.ok()
                .and_then(|m| m.body().deserialize::<(String, String, String)>().ok())
                .filter(|(_, _, new)| new.is_empty())
                .map_or(Input::Ignore, |(name, old, _)| Input::OwnerGone(if old.is_empty() { name } else { old }))
        });
        let mut foreign: Vec<Input> = Vec::new();
        let foreign_signals = if owns_watcher {
            None
        } else {
            let proxy = zbus::Proxy::new(&connection, WATCHER_NAME, WATCHER_PATH, WATCHER_NAME).await?;
            let _ = proxy.call::<_, _, ()>("RegisterStatusNotifierHost", &(host_name.as_str(),)).await;
            if let Ok(existing) = proxy.get_property::<Vec<String>>("RegisteredStatusNotifierItems").await {
                foreign.extend(existing.into_iter().map(Input::Register));
            }
            let registered_stream = proxy
                .receive_signal("StatusNotifierItemRegistered")
                .await?
                .map(|m| m.body().deserialize::<String>().map_or(Input::Ignore, Input::Register));
            let unregistered_stream = proxy
                .receive_signal("StatusNotifierItemUnregistered")
                .await?
                .map(|m| m.body().deserialize::<String>().map_or(Input::Ignore, Input::Unregister));
            Some(registered_stream.or(unregistered_stream))
        };
        events.send(Event::Watcher(true));
        for input in foreign {
            let _ = input_tx.send(input).await;
        }

        let watcher_signals: std::pin::Pin<Box<dyn futures_lite::Stream<Item = Input>>> = match foreign_signals {
            Some(stream) => Box::pin(stream.chain(futures_lite::stream::pending::<Input>())),
            None => Box::pin(futures_lite::stream::pending::<Input>()),
        };
        let cmds = commands.map(Input::Command);
        let mut stream =
            std::pin::pin!(input_rx.or(cmds).or(item_signals).or(menu_signals).or(owner_changes).or(watcher_signals));
        let emitter = SignalEmitter::new(&connection, WATCHER_PATH)?;

        let mut items: Vec<Tracked> = Vec::new();
        let mut last: Vec<TrayItem> = Vec::new();
        let mut open_menu: Option<String> = None;
        while let Some(input) = stream.next().await {
            match input {
                Input::Register(raw) => {
                    let (bus_name, path) = item_address(&raw, "");
                    let key = format!("{bus_name}{path}");
                    if items.iter().any(|t| t.key == key) {
                        continue;
                    }
                    if let Some(tracked) = track(&connection, &raw).await {
                        items.push(tracked);
                        publish(&events, &items, &mut last);
                    }
                }
                Input::Unregister(raw) => {
                    let (bus_name, path) = item_address(&raw, "");
                    let key = format!("{bus_name}{path}");
                    remove_items(&mut items, |t| t.key == key, &registered, &emitter, owns_watcher).await;
                    publish(&events, &items, &mut last);
                }
                Input::OwnerGone(name) => {
                    remove_items(&mut items, |t| t.owner == name || t.bus == name, &registered, &emitter, owns_watcher)
                        .await;
                    publish(&events, &items, &mut last);
                }
                Input::ItemSignal(owner) => {
                    let mut changed = false;
                    for tracked in items.iter_mut().filter(|t| t.owner == owner) {
                        if let Ok(item) = fetch_item(&connection, &tracked.bus, &tracked.path, &tracked.key).await {
                            tracked.item = item;
                            changed = true;
                        }
                    }
                    if changed {
                        publish(&events, &items, &mut last);
                    }
                }
                Input::MenuSignal(owner) => {
                    let open = open_menu.as_ref().and_then(|key| items.iter().find(|t| t.key == *key));
                    if let Some(tracked) = open.filter(|t| t.owner == owner) {
                        match fetch_menu(&connection, tracked, false).await {
                            Ok(entries) => {
                                events.send(Event::Menu(TrayMenu { key: tracked.key.clone(), entries }));
                            }
                            Err(e) => tracing::debug!("tray menu refresh failed: {e}"),
                        }
                    }
                }
                Input::Command(command) => {
                    match &command {
                        TrayCommand::LoadMenu(key) => open_menu = Some(key.clone()),
                        TrayCommand::CloseMenu => open_menu = None,
                        _ => {}
                    }
                    run_command(&connection, &items, command, &events).await
                }
                Input::Ignore => {}
            }
        }
        Ok(())
    }
    .await;
    if let Err(e) = result {
        tracing::info!("tray unavailable: {e}");
        events.send(Event::Unavailable);
    }
}

async fn remove_items(
    items: &mut Vec<Tracked>,
    matches: impl Fn(&Tracked) -> bool,
    registered: &Arc<Mutex<Vec<String>>>,
    emitter: &SignalEmitter<'_>,
    owns_watcher: bool,
) {
    let gone: Vec<String> = items.iter().filter(|t| matches(t)).map(|t| t.key.clone()).collect();
    items.retain(|t| !matches(t));
    for key in gone {
        if owns_watcher {
            if let Ok(mut list) = registered.lock() {
                list.retain(|k| *k != key);
            }
            let _ = Watcher::status_notifier_item_unregistered(emitter, &key).await;
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::mpsc;
    use std::time::{Duration, Instant};

    use calloop::EventLoop;

    use super::*;
    use crate::platform::dbus::testbus::TestBus;

    fn props(entries: &[(&str, ZValue<'static>)]) -> Props {
        entries.iter().map(|(k, v)| ((*k).to_owned(), OwnedValue::try_from(v.clone()).unwrap())).collect()
    }

    #[test]
    fn service_strings_resolve_to_bus_and_path() {
        assert_eq!(
            item_address("/org/ayatana/NotificationItem/x", ":1.5"),
            (":1.5".into(), "/org/ayatana/NotificationItem/x".into())
        );
        assert_eq!(item_address(":1.7/Custom/Item", ":1.5"), (":1.7".into(), "/Custom/Item".into()));
        assert_eq!(item_address("org.example.App", ":1.5"), ("org.example.App".into(), DEFAULT_ITEM_PATH.into()));
    }

    #[test]
    fn argb_bytes_become_rgba() {
        assert_eq!(argb_to_rgba(&[0xFF, 1, 2, 3, 0x80, 4, 5, 6]), vec![1, 2, 3, 0xFF, 4, 5, 6, 0x80]);
    }

    #[test]
    fn the_largest_valid_pixmap_wins() {
        let pixmaps: Pixmaps = vec![(1, 1, vec![255, 0, 0, 0]), (2, 1, vec![0; 8]), (2, 2, vec![0; 3])];
        let best = best_pixmap(&pixmaps).unwrap();
        assert_eq!((best.width, best.height), (2, 1));
        assert!(best_pixmap(&Vec::new()).is_none());
    }

    #[test]
    fn mnemonics_are_stripped() {
        assert_eq!(strip_mnemonic("_Open"), "Open");
        assert_eq!(strip_mnemonic("Save __as"), "Save _as");
        assert_eq!(strip_mnemonic("Plain"), "Plain");
    }

    #[test]
    fn item_properties_map_to_a_tray_item() {
        let p = props(&[
            ("Id", ZValue::from("app")),
            ("Title", ZValue::from("App")),
            ("Status", ZValue::from("NeedsAttention")),
            ("IconName", ZValue::from("app-icon")),
            ("Menu", ZValue::from(zbus::ObjectPath::from_static_str_unchecked("/Menu"))),
            ("ItemIsMenu", ZValue::from(true)),
            ("IconPixmap", ZValue::from(vec![(1i32, 1i32, vec![255u8, 9, 8, 7])])),
        ]);
        let item = build_item("k", &p);
        assert_eq!(item.id, "app");
        assert_eq!(item.status, TrayStatus::NeedsAttention);
        assert_eq!(item.icon, "app-icon");
        assert_eq!(item.menu_path.as_deref(), Some("/Menu"));
        assert!(item.is_menu_only && item.has_menu());
        assert_eq!(item.pixmap.as_ref().unwrap().rgba, vec![9, 8, 7, 255]);
        assert_eq!(item.display_name(), "App");
    }

    #[test]
    fn theme_path_icons_resolve_to_files() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("custom.png"), b"x").unwrap();
        let theme = dir.path().to_string_lossy().into_owned();
        assert_eq!(resolve_icon("custom", &theme), dir.path().join("custom.png").to_string_lossy());
        assert_eq!(resolve_icon("missing", &theme), "missing");
        assert_eq!(resolve_icon("custom", ""), "custom");
    }

    fn node(id: i32, entries: &[(&str, ZValue<'static>)], children: Vec<OwnedValue>) -> Layout {
        (id, props(entries), children)
    }

    fn as_child(layout: Layout) -> OwnedValue {
        OwnedValue::try_from(ZValue::from(layout)).unwrap()
    }

    #[test]
    fn menu_layout_parses_nested_entries() {
        let sub = node(3, &[("label", ZValue::from("_Sub"))], Vec::new());
        let root = node(
            0,
            &[],
            vec![
                as_child(node(1, &[("label", ZValue::from("Open")), ("enabled", ZValue::from(false))], Vec::new())),
                as_child(node(2, &[("type", ZValue::from("separator"))], Vec::new())),
                as_child(node(4, &[("label", ZValue::from("Hidden")), ("visible", ZValue::from(false))], Vec::new())),
                as_child(node(
                    5,
                    &[
                        ("label", ZValue::from("More")),
                        ("toggle-type", ZValue::from("checkmark")),
                        ("toggle-state", ZValue::from(1i32)),
                    ],
                    vec![as_child(sub)],
                )),
            ],
        );
        let entries = parse_layout(&root);
        assert_eq!(entries.len(), 3);
        assert_eq!(entries[0].text, "Open");
        assert!(!entries[0].enabled);
        assert!(entries[1].separator);
        assert_eq!(entries[2].toggle, MenuToggle::Check);
        assert!(entries[2].checked && entries[2].has_children());
        assert_eq!(entries[2].children[0].text, "Sub");
    }

    struct FakeItem {
        events: mpsc::Sender<String>,
    }

    #[interface(name = "org.kde.StatusNotifierItem")]
    impl FakeItem {
        #[zbus(property)]
        fn id(&self) -> String {
            "fake".into()
        }
        #[zbus(property)]
        fn title(&self) -> String {
            "Fake".into()
        }
        #[zbus(property)]
        fn status(&self) -> String {
            "Active".into()
        }
        #[zbus(property)]
        fn icon_name(&self) -> String {
            "fake-icon".into()
        }
        #[zbus(property)]
        fn menu(&self) -> zbus::OwnedObjectPath {
            zbus::OwnedObjectPath::try_from("/Menu").unwrap()
        }
        #[zbus(property)]
        fn tool_tip(&self) -> (String, Pixmaps, String, String) {
            ("tip-icon".into(), vec![(1, 1, vec![255, 1, 2, 3])], "Tip".into(), "Body".into())
        }
        fn activate(&self, x: i32, y: i32) {
            let _ = self.events.send(format!("activate {x} {y}"));
        }
        fn scroll(&self, delta: i32, orientation: String) {
            let _ = self.events.send(format!("scroll {delta} {orientation}"));
        }
    }

    struct FakeMenu {
        events: mpsc::Sender<String>,
        renamed: bool,
    }

    #[interface(name = "com.canonical.dbusmenu")]
    impl FakeMenu {
        fn about_to_show(&self, _id: i32) -> bool {
            false
        }
        fn get_layout(&self, _parent: i32, _depth: i32, _names: Vec<String>) -> (u32, Layout) {
            let second = if self.renamed { "Preferences" } else { "Settings" };
            let child =
                |id: i32, label: &str| as_child(node(id, &[("label", ZValue::from(label.to_owned()))], Vec::new()));
            (1, node(0, &[], vec![child(1, "_Quit"), child(2, second)]))
        }
        async fn event(
            &mut self,
            id: i32,
            kind: String,
            _data: OwnedValue,
            _timestamp: u32,
            #[zbus(signal_emitter)] emitter: SignalEmitter<'_>,
        ) {
            let _ = self.events.send(format!("event {id} {kind}"));
            self.renamed = true;
            let _ = Self::layout_updated(&emitter, 2, 0).await;
        }

        #[zbus(signal)]
        async fn layout_updated(emitter: &SignalEmitter<'_>, revision: u32, parent: i32) -> zbus::Result<()>;
    }

    fn pump_until(event_loop: &mut EventLoop<'static, ()>, done: impl Fn() -> bool) -> bool {
        let deadline = Instant::now() + Duration::from_secs(10);
        while !done() && Instant::now() < deadline {
            event_loop.dispatch(Duration::from_millis(10), &mut ()).unwrap();
        }
        done()
    }

    #[test]
    fn items_register_update_and_answer_commands() {
        let bus = TestBus::start();
        let mut event_loop: EventLoop<'static, ()> = EventLoop::try_new().unwrap();
        let tray = Tray::start(&event_loop.handle(), bus.kind()).unwrap();
        let state = tray.state.clone();
        assert!(pump_until(&mut event_loop, || state.ready.get_untracked()));
        assert!(state.available.get_untracked());

        let (event_tx, event_rx) = mpsc::channel();
        let (ready_tx, ready_rx) = mpsc::channel();
        let (stop_tx, stop_rx) = mpsc::channel::<()>();
        let address = bus.address.clone();
        let handle = std::thread::spawn(move || {
            async_io::block_on(async move {
                let connection = zbus::connection::Builder::address(address.as_str())
                    .serve_at(DEFAULT_ITEM_PATH, FakeItem { events: event_tx.clone() })
                    .serve_at("/Menu", FakeMenu { events: event_tx, renamed: false })
                    .build()
                    .await
                    .unwrap();
                let watcher = zbus::Proxy::new(&connection, WATCHER_NAME, WATCHER_PATH, WATCHER_NAME).await.unwrap();
                watcher.call::<_, _, ()>("RegisterStatusNotifierItem", &(DEFAULT_ITEM_PATH,)).await.unwrap();
                ready_tx.send(()).unwrap();
                while stop_rx.try_recv().is_err() {
                    async_io::Timer::after(Duration::from_millis(20)).await;
                }
                drop(connection);
            });
        });
        ready_rx.recv_timeout(Duration::from_secs(10)).unwrap();

        assert!(pump_until(&mut event_loop, || state.items.get_untracked().len() == 1));
        let item = state.item_by_id("fake").unwrap();
        assert_eq!(item.title, "Fake");
        assert_eq!(item.icon, "fake-icon");
        assert_eq!(item.menu_path.as_deref(), Some("/Menu"));
        assert_eq!((item.tooltip_icon.as_str(), item.tooltip_title.as_str()), ("tip-icon", "Tip"));
        assert_eq!(item.tooltip_text, "Body");
        assert_eq!(item.tooltip_pixmap.unwrap().rgba, vec![1, 2, 3, 255]);

        assert!(tray.activate(&item.key, 4, 5));
        assert_eq!(event_rx.recv_timeout(Duration::from_secs(10)).unwrap(), "activate 4 5");
        assert!(tray.scroll(&item.key, -1, true));
        assert_eq!(event_rx.recv_timeout(Duration::from_secs(10)).unwrap(), "scroll -1 horizontal");

        assert!(tray.load_menu(&item.key));
        assert!(pump_until(&mut event_loop, || state.menu.get_untracked().is_some()));
        let menu = state.menu.get_untracked().unwrap();
        assert_eq!(menu.entries.iter().map(|e| e.text.as_str()).collect::<Vec<_>>(), ["Quit", "Settings"]);
        assert!(tray.click_menu_entry(&item.key, 1));
        assert_eq!(event_rx.recv_timeout(Duration::from_secs(10)).unwrap(), "event 1 clicked");
        assert!(pump_until(&mut event_loop, || {
            state.menu.get_untracked().is_some_and(|m| m.entries.iter().any(|e| e.text == "Preferences"))
        }));
        tray.close_menu();
        assert!(state.menu.get_untracked().is_none());

        stop_tx.send(()).unwrap();
        handle.join().unwrap();
        assert!(pump_until(&mut event_loop, || state.items.get_untracked().is_empty()));
    }
}
