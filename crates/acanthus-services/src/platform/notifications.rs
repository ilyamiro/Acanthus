use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use acanthus_core::{Signal, batch};
use acanthus_io::IoError;
use calloop::LoopHandle;
use futures_lite::StreamExt;
use serde_json::Value;
use zbus::object_server::SignalEmitter;
use zbus::{OwnedVariant, interface};

pub use crate::kit::Urgency;
use crate::kit::{Notice, Notifier, NotifySendNotifier};
use crate::platform::bridge::{EventSink, ServiceHandle, spawn_supervised};
use crate::platform::dbus::{BusKind, variant_to_json};

const WELL_KNOWN_NAME: &str = "org.freedesktop.Notifications";
const OBJECT_PATH: &str = "/org/freedesktop/Notifications";

/// Why a notification was closed, with the codes of the specification.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CloseReason {
    /// It timed out.
    Expired,
    /// The user dismissed it.
    Dismissed,
    /// The sender or the service closed it.
    Closed,
    /// Another reason.
    Undefined,
}

impl CloseReason {
    /// The numeric code of the specification.
    pub fn code(self) -> u32 {
        match self {
            CloseReason::Expired => 1,
            CloseReason::Dismissed => 2,
            CloseReason::Closed => 3,
            CloseReason::Undefined => 4,
        }
    }
}

/// A button of a notification.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Action {
    /// Identifier sent back when it is invoked.
    pub id: String,
    /// Label.
    pub text: String,
}

/// Raw image data from the `image-data` hint.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct ImageData {
    /// Width in pixels.
    pub width: i32,
    /// Height in pixels.
    pub height: i32,
    /// Bytes per row.
    pub rowstride: i32,
    /// True when there is an alpha channel.
    pub has_alpha: bool,
    /// Bits per colour sample; only 8 is supported.
    pub bits_per_sample: i32,
    /// Channels per pixel, 3 or 4.
    pub channels: i32,
    /// The pixels.
    pub data: Vec<u8>,
}

impl ImageData {
    /// The pixels as tightly packed RGBA, or `None` when the format is not supported.
    pub fn to_rgba(&self) -> Option<Vec<u8>> {
        if self.bits_per_sample != 8 || self.width <= 0 || self.height <= 0 || !(3..=4).contains(&self.channels) {
            return None;
        }
        let (width, height, channels) = (self.width as usize, self.height as usize, self.channels as usize);
        let stride = if self.rowstride > 0 { self.rowstride as usize } else { width * channels };
        if stride < width * channels || self.data.len() < stride * (height - 1) + width * channels {
            return None;
        }
        let mut out = Vec::with_capacity(width * height * 4);
        for row in 0..height {
            for column in 0..width {
                let at = row * stride + column * channels;
                out.extend_from_slice(&self.data[at..at + 3]);
                out.push(if channels == 4 && self.has_alpha { self.data[at + 3] } else { 255 });
            }
        }
        Some(out)
    }
}

/// A notification as received from an application.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Notification {
    /// Identifier assigned by the server.
    pub id: u32,
    /// Name of the sending application.
    pub app_name: String,
    /// Icon of the application.
    pub app_icon: String,
    /// Title.
    pub summary: String,
    /// Body text.
    pub body: String,
    /// Buttons.
    pub actions: Vec<Action>,
    /// Urgency.
    pub urgency: Urgency,
    /// Timeout in milliseconds; -1 for the server's default, 0 for none.
    pub expire_timeout_ms: i32,
    /// Path of an image to show.
    pub image_path: String,
    /// Raw image data to show.
    pub image_data: Option<ImageData>,
    /// Desktop entry of the sender.
    pub desktop_entry: String,
    /// Category hint.
    pub category: String,
    /// True when the notification should not be kept in history.
    pub transient: bool,
    /// True when it stays after an action is invoked.
    pub resident: bool,
    /// Arrival time as Unix milliseconds.
    pub timestamp_ms: u64,
    /// True when it replaced an earlier notification.
    pub replaced: bool,
}

impl Notification {
    /// The image to show: the image path, else the application icon.
    pub fn image(&self) -> &str {
        if !self.image_path.is_empty() { &self.image_path } else { &self.app_icon }
    }
}

/// Pairs `[id, text, id, text, ...]` as sent over D-Bus into actions.
pub fn parse_actions(raw: &[String]) -> Vec<Action> {
    raw.chunks(2)
        .filter(|pair| pair.len() == 2)
        .map(|pair| Action { id: pair[0].clone(), text: pair[1].clone() })
        .collect()
}

fn parse_image_data(value: &OwnedVariant) -> Option<ImageData> {
    let json = variant_to_json(value);
    let fields = json.as_array()?;
    let int = |i: usize| fields.get(i).and_then(Value::as_i64).map(|n| n as i32);
    let bytes: Vec<u8> = fields.get(6)?.as_array()?.iter().filter_map(|b| b.as_u64().map(|n| n as u8)).collect();
    Some(ImageData {
        width: int(0)?,
        height: int(1)?,
        rowstride: int(2)?,
        has_alpha: fields.get(3)?.as_bool()?,
        bits_per_sample: int(4)?,
        channels: int(5)?,
        data: bytes,
    })
}

/// Builds a notification from the arguments and hints of a `Notify` call.
#[allow(clippy::too_many_arguments)]
pub fn build_notification(
    id: u32,
    app_name: &str,
    app_icon: &str,
    summary: &str,
    body: &str,
    actions: &[String],
    hints: &HashMap<String, OwnedVariant>,
    expire_timeout: i32,
    replaced: bool,
) -> Notification {
    let hint_json = |key: &str| hints.get(key).map(|v| variant_to_json(v));
    let hint_text = |key: &str| hint_json(key).and_then(|v| v.as_str().map(str::to_owned)).unwrap_or_default();
    let hint_flag = |key: &str| hint_json(key).and_then(|v| v.as_bool()).unwrap_or(false);
    let image_data =
        ["image-data", "image_data", "icon_data"].iter().find_map(|k| hints.get(*k).and_then(parse_image_data));
    let image_path =
        [hint_text("image-path"), hint_text("image_path")].into_iter().find(|s| !s.is_empty()).unwrap_or_default();
    Notification {
        id,
        app_name: app_name.to_owned(),
        app_icon: app_icon.to_owned(),
        summary: summary.to_owned(),
        body: body.to_owned(),
        actions: parse_actions(actions),
        urgency: hint_json("urgency").and_then(|v| v.as_u64()).map_or(Urgency::Normal, |n| Urgency::from_hint(n as u8)),
        expire_timeout_ms: expire_timeout,
        image_path,
        image_data,
        desktop_entry: hint_text("desktop-entry"),
        category: hint_text("category"),
        transient: hint_flag("transient"),
        resident: hint_flag("resident"),
        timestamp_ms: SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_millis() as u64),
        replaced,
    }
}

enum Event {
    Name(bool),
    Notified(Box<Notification>),
    Closed(u32),
}

/// A request to the notification worker.
pub enum Command {
    /// Close the notification with this id for this reason.
    Close(u32, CloseReason),
    /// Invoke the action with this id on the notification with this id.
    Invoke(u32, String),
}

struct Daemon {
    events: EventSink<Event>,
    next_id: Arc<AtomicU32>,
    live: Arc<std::sync::Mutex<Vec<u32>>>,
    app: String,
}

#[interface(name = "org.freedesktop.Notifications")]
impl Daemon {
    fn get_capabilities(&self) -> Vec<String> {
        ["body", "actions", "icon-static", "persistence", "action-icons"].iter().map(|s| (*s).to_owned()).collect()
    }

    #[allow(clippy::too_many_arguments)]
    fn notify(
        &self,
        app_name: String,
        replaces_id: u32,
        app_icon: String,
        summary: String,
        body: String,
        actions: Vec<String>,
        hints: HashMap<String, OwnedVariant>,
        expire_timeout: i32,
    ) -> u32 {
        let replaced = replaces_id != 0 && self.live.lock().is_ok_and(|l| l.contains(&replaces_id));
        let id = if replaced { replaces_id } else { self.next_id.fetch_add(1, Ordering::Relaxed) };
        if let Ok(mut live) = self.live.lock()
            && !live.contains(&id)
        {
            live.push(id);
        }
        let notification =
            build_notification(id, &app_name, &app_icon, &summary, &body, &actions, &hints, expire_timeout, replaced);
        self.events.send(Event::Notified(Box::new(notification)));
        id
    }

    async fn close_notification(&self, id: u32, #[zbus(signal_emitter)] emitter: SignalEmitter<'_>) {
        let known = self.live.lock().is_ok_and(|mut l| {
            let before = l.len();
            l.retain(|n| *n != id);
            l.len() != before
        });
        if known {
            let _ = Daemon::notification_closed(&emitter, id, CloseReason::Closed.code()).await;
            self.events.send(Event::Closed(id));
        }
    }

    fn get_server_information(&self) -> (String, String, String, String) {
        (self.app.clone(), self.app.clone(), env!("CARGO_PKG_VERSION").into(), "1.2".into())
    }

    #[zbus(signal)]
    async fn notification_closed(emitter: &SignalEmitter<'_>, id: u32, reason: u32) -> zbus::Result<()>;

    #[zbus(signal)]
    async fn action_invoked(emitter: &SignalEmitter<'_>, id: u32, action_key: &str) -> zbus::Result<()>;
}

async fn run(
    bus: BusKind,
    app: String,
    next_id: Arc<AtomicU32>,
    events: EventSink<Event>,
    commands: async_channel::Receiver<Command>,
) {
    let result: zbus::Result<()> = async {
        let connection = bus.connect().await?;
        let daemon = Daemon { events: events.clone(), next_id, live: Arc::default(), app };
        let live = daemon.live.clone();
        connection.object_server().at(OBJECT_PATH, daemon).await?;
        match connection.request_name_with_flags(WELL_KNOWN_NAME, zbus::fdo::RequestNameFlags::DO_NOT_QUEUE).await {
            Ok(_) => {
                events.send(Event::Name(true));
            }
            Err(e) => {
                tracing::warn!("cannot own {WELL_KNOWN_NAME}: {e}");
                events.send(Event::Name(false));
                return Ok(());
            }
        }
        let emitter = SignalEmitter::new(&connection, OBJECT_PATH)?;
        let mut commands = std::pin::pin!(commands);
        while let Some(command) = commands.next().await {
            match command {
                Command::Close(id, reason) => {
                    let known = live.lock().is_ok_and(|mut l| {
                        let before = l.len();
                        l.retain(|n| *n != id);
                        l.len() != before
                    });
                    if known {
                        let _ = Daemon::notification_closed(&emitter, id, reason.code()).await;
                    }
                }
                Command::Invoke(id, key) => {
                    let _ = Daemon::action_invoked(&emitter, id, &key).await;
                }
            }
        }
        Ok(())
    }
    .await;
    if let Err(e) = result {
        tracing::warn!("notification server stopped: {e}");
        events.send(Event::Name(false));
    }
}

type Listener = Box<dyn FnMut(&Notification)>;

fn publish(live: &Signal<Vec<Notification>>, listeners: &Rc<RefCell<Vec<Listener>>>, notification: &Notification) {
    live.update(|list| match list.iter_mut().find(|n| n.id == notification.id) {
        Some(existing) => *existing = notification.clone(),
        None => list.push(notification.clone()),
    });
    for listener in listeners.borrow_mut().iter_mut() {
        listener(notification);
    }
}
type CloseListener = Box<dyn FnMut(u32)>;
type ActionHandler = Box<dyn FnOnce(String)>;
type LocalActions = Rc<RefCell<HashMap<u32, ActionHandler>>>;

/// The `org.freedesktop.Notifications` server: receives notifications from applications and publishes them to listeners.
pub struct NotificationServer {
    /// True while the server owns the bus name.
    pub running: Signal<bool>,
    /// The notifications currently shown.
    pub live: Signal<Vec<Notification>>,
    listeners: Rc<RefCell<Vec<Listener>>>,
    close_listeners: Rc<RefCell<Vec<CloseListener>>>,
    next_id: Arc<AtomicU32>,
    local_actions: LocalActions,
    service: ServiceHandle<Command>,
}

impl NotificationServer {
    /// Starts the server for the application `app` on `bus`.
    pub fn start<D: 'static>(
        handle: &LoopHandle<'static, D>,
        bus: BusKind,
        app: &str,
    ) -> Result<NotificationServer, IoError> {
        let app = app.to_owned();
        let running = Signal::new(false);
        let live: Signal<Vec<Notification>> = Signal::new(Vec::new());
        let listeners: Rc<RefCell<Vec<Listener>>> = Rc::default();
        let close_listeners: Rc<RefCell<Vec<CloseListener>>> = Rc::default();
        let next_id = Arc::new(AtomicU32::new(1));
        let local_actions: LocalActions = Rc::default();
        let (r, l, ls, cl, la) =
            (running.clone(), live.clone(), listeners.clone(), close_listeners.clone(), local_actions.clone());
        let worker_ids = next_id.clone();
        let service = spawn_supervised(
            handle,
            "acanthus-notifications",
            move |event| match event {
                Event::Name(owned) => r.set(owned),
                Event::Notified(notification) => publish(&l, &ls, &notification),
                Event::Closed(id) => {
                    la.borrow_mut().remove(&id);
                    batch(|| l.update(|list| list.retain(|n| n.id != id)));
                    for listener in cl.borrow_mut().iter_mut() {
                        listener(id);
                    }
                }
            },
            move |events, commands| run(bus.clone(), app.clone(), worker_ids.clone(), events, commands),
        )?;
        Ok(NotificationServer { running, live, listeners, close_listeners, next_id, local_actions, service })
    }

    /// Adds a listener called for every new notification.
    pub fn on_notification(&self, listener: impl FnMut(&Notification) + 'static) {
        self.listeners.borrow_mut().push(Box::new(listener));
    }

    /// Adds a listener called with the id of every closed notification.
    pub fn on_closed(&self, listener: impl FnMut(u32) + 'static) {
        self.close_listeners.borrow_mut().push(Box::new(listener));
    }

    /// Shows a notification created by this process and returns its id.
    pub fn post_local(&self, notice: Notice) -> u32 {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let Notice { app_name, summary, body, icon, urgency, actions, on_action } = notice;
        let notification = Notification {
            id,
            app_name,
            app_icon: icon,
            summary,
            body,
            actions: actions.into_iter().map(|(id, text)| Action { id, text }).collect(),
            urgency,
            expire_timeout_ms: -1,
            timestamp_ms: SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_millis() as u64),
            ..Notification::default()
        };
        if let Some(handler) = on_action {
            self.local_actions.borrow_mut().insert(id, handler);
        }
        publish(&self.live, &self.listeners, &notification);
        id
    }

    /// Closes a notification and tells its sender.
    pub fn close(&self, id: u32, reason: CloseReason) {
        self.local_actions.borrow_mut().remove(&id);
        self.live.update(|list| list.retain(|n| n.id != id));
        self.service.send(Command::Close(id, reason));
    }

    /// Invokes an action: the local handler for local notifications, else a signal to the sender.
    pub fn invoke_action(&self, id: u32, action_id: &str) {
        let local = self.local_actions.borrow_mut().remove(&id);
        if let Some(handler) = local {
            handler(action_id.to_owned());
            return;
        }
        self.service.send(Command::Invoke(id, action_id.to_owned()));
    }
}

/// A [`Notifier`] that shows notices through the local server when it runs and falls back to `notify-send` otherwise.
pub struct LocalNotifier {
    server: Rc<NotificationServer>,
    fallback: NotifySendNotifier,
}

impl LocalNotifier {
    /// Creates a notifier using `server`, or `fallback` while the server is not running.
    pub fn new(server: Rc<NotificationServer>, fallback: NotifySendNotifier) -> LocalNotifier {
        LocalNotifier { server, fallback }
    }
}

impl Notifier for LocalNotifier {
    fn notify(&self, notice: Notice) {
        if self.server.running.get_untracked() {
            self.server.post_local(notice);
        } else {
            self.fallback.notify(notice);
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::mpsc;
    use std::time::{Duration, Instant};

    use calloop::EventLoop;
    use zbus::Variant;

    use super::*;
    use crate::platform::dbus::testbus::TestBus;

    fn hints(pairs: Vec<(&str, Variant<'static>)>) -> HashMap<String, OwnedVariant> {
        pairs.into_iter().map(|(k, v)| (k.to_owned(), OwnedVariant::try_from(v).unwrap())).collect()
    }

    #[test]
    fn actions_pair_up_identifiers_and_labels() {
        let raw: Vec<String> =
            ["default", "Open", "dismiss", "Dismiss", "dangling"].iter().map(|s| (*s).to_owned()).collect();
        let actions = parse_actions(&raw);
        assert_eq!(
            actions,
            [
                Action { id: "default".into(), text: "Open".into() },
                Action { id: "dismiss".into(), text: "Dismiss".into() }
            ]
        );
    }

    #[test]
    fn hints_map_to_urgency_images_and_flags() {
        let n = build_notification(
            4,
            "app",
            "icon",
            "sum",
            "body",
            &[],
            &hints(vec![
                ("urgency", Variant::from(2u8)),
                ("image-path", Variant::from("/tmp/x.png")),
                ("desktop-entry", Variant::from("firefox")),
                ("transient", Variant::from(true)),
            ]),
            5000,
            false,
        );
        assert_eq!(n.urgency, Urgency::Critical);
        assert_eq!(n.image(), "/tmp/x.png");
        assert_eq!((n.desktop_entry.as_str(), n.transient, n.resident), ("firefox", true, false));
        let plain = build_notification(1, "a", "ic", "s", "", &[], &HashMap::new(), -1, false);
        assert_eq!((plain.urgency, plain.image()), (Urgency::Normal, "ic"));
    }

    #[test]
    fn raw_image_data_converts_to_rgba_honouring_rowstride() {
        let image = ImageData {
            width: 2,
            height: 2,
            rowstride: 8,
            has_alpha: false,
            bits_per_sample: 8,
            channels: 3,
            data: vec![1, 2, 3, 4, 5, 6, 0, 0, 7, 8, 9, 10, 11, 12, 0, 0],
        };
        assert_eq!(image.to_rgba().unwrap(), [1, 2, 3, 255, 4, 5, 6, 255, 7, 8, 9, 255, 10, 11, 12, 255]);
        let alpha = ImageData {
            width: 1,
            height: 1,
            rowstride: 4,
            has_alpha: true,
            bits_per_sample: 8,
            channels: 4,
            data: vec![9, 8, 7, 6],
        };
        assert_eq!(alpha.to_rgba().unwrap(), [9, 8, 7, 6]);
        assert!(ImageData { bits_per_sample: 16, ..alpha.clone() }.to_rgba().is_none());
        assert!(ImageData { data: vec![1], ..alpha }.to_rgba().is_none());
    }

    #[zbus::proxy(
        interface = "org.freedesktop.Notifications",
        default_service = "org.freedesktop.Notifications",
        default_path = "/org/freedesktop/Notifications"
    )]
    trait Client {
        #[allow(clippy::too_many_arguments)]
        fn notify(
            &self,
            app: &str,
            replaces: u32,
            icon: &str,
            summary: &str,
            body: &str,
            actions: Vec<&str>,
            hints: HashMap<&str, Variant<'_>>,
            timeout: i32,
        ) -> zbus::Result<u32>;
        fn get_capabilities(&self) -> zbus::Result<Vec<String>>;
        fn close_notification(&self, id: u32) -> zbus::Result<()>;
        #[zbus(signal)]
        fn notification_closed(&self, id: u32, reason: u32) -> zbus::Result<()>;
        #[zbus(signal)]
        fn action_invoked(&self, id: u32, key: String) -> zbus::Result<()>;
    }

    enum Call {
        Notify { replaces: u32, summary: &'static str, urgency: u8 },
        Capabilities,
        Close(u32),
    }

    fn run_client(address: String, calls: mpsc::Receiver<Call>, results: mpsc::Sender<String>) {
        std::thread::spawn(move || {
            async_io::block_on(async move {
                let connection = zbus::connection::Builder::address(address.as_str()).build().await.unwrap();
                let proxy = ClientProxy::new(&connection).await.unwrap();
                let mut closed = proxy.receive_notification_closed().await.unwrap();
                let mut invoked = proxy.receive_action_invoked().await.unwrap();
                results.send("ready".into()).unwrap();
                loop {
                    let call = async_io::block_on(async { calls.recv().ok() });
                    match call {
                        Some(Call::Notify { replaces, summary, urgency }) => {
                            let mut hints = HashMap::new();
                            hints.insert("urgency", Variant::from(urgency));
                            let id = proxy
                                .notify(
                                    "test-app",
                                    replaces,
                                    "dialog-info",
                                    summary,
                                    "body text",
                                    vec!["default", "Open"],
                                    hints,
                                    3000,
                                )
                                .await
                                .unwrap();
                            results.send(format!("id {id}")).unwrap();
                        }
                        Some(Call::Capabilities) => {
                            results
                                .send(format!("caps {}", proxy.get_capabilities().await.unwrap().join(",")))
                                .unwrap();
                        }
                        Some(Call::Close(id)) => {
                            proxy.close_notification(id).await.unwrap();
                            let signal = closed.next().await.unwrap();
                            let args = signal.args().unwrap();
                            results.send(format!("closed {} {}", args.id, args.reason)).unwrap();
                        }
                        None => return,
                    }
                    if let Some(signal) = futures_lite::future::poll_once(invoked.next()).await.flatten() {
                        let args = signal.args().unwrap();
                        results.send(format!("invoked {} {}", args.id, args.key)).unwrap();
                    }
                }
            });
        });
    }

    fn pump_until(event_loop: &mut EventLoop<'static, ()>, done: impl Fn() -> bool) -> bool {
        let deadline = Instant::now() + Duration::from_secs(10);
        while !done() && Instant::now() < deadline {
            event_loop.dispatch(Duration::from_millis(10), &mut ()).unwrap();
        }
        done()
    }

    fn recv(event_loop: &mut EventLoop<'static, ()>, results: &mpsc::Receiver<String>) -> String {
        let deadline = Instant::now() + Duration::from_secs(10);
        while Instant::now() < deadline {
            event_loop.dispatch(Duration::from_millis(5), &mut ()).unwrap();
            if let Ok(message) = results.try_recv() {
                return message;
            }
        }
        panic!("client produced no result");
    }

    #[test]
    fn full_notification_lifecycle_over_a_real_bus() {
        let bus = TestBus::start();
        let mut event_loop: EventLoop<'static, ()> = EventLoop::try_new().unwrap();
        let server = NotificationServer::start(&event_loop.handle(), bus.kind(), "demo").unwrap();
        let received = Rc::new(RefCell::new(Vec::<Notification>::new()));
        let closed_ids = Rc::new(RefCell::new(Vec::<u32>::new()));
        let (r, c) = (received.clone(), closed_ids.clone());
        server.on_notification(move |n| r.borrow_mut().push(n.clone()));
        server.on_closed(move |id| c.borrow_mut().push(id));
        let running = server.running.clone();
        assert!(pump_until(&mut event_loop, || running.get_untracked()));

        let (call_tx, call_rx) = mpsc::channel();
        let (result_tx, result_rx) = mpsc::channel();
        run_client(bus.address.clone(), call_rx, result_tx);
        assert_eq!(recv(&mut event_loop, &result_rx), "ready");

        call_tx.send(Call::Capabilities).unwrap();
        assert!(recv(&mut event_loop, &result_rx).starts_with("caps body,actions"));

        call_tx.send(Call::Notify { replaces: 0, summary: "Hello", urgency: 2 }).unwrap();
        assert_eq!(recv(&mut event_loop, &result_rx), "id 1");
        assert!(pump_until(&mut event_loop, || received.borrow().len() == 1));
        let first = received.borrow()[0].clone();
        assert_eq!(
            (first.summary.as_str(), first.app_name.as_str(), first.urgency, first.expire_timeout_ms),
            ("Hello", "test-app", Urgency::Critical, 3000)
        );
        assert_eq!(first.actions, [Action { id: "default".into(), text: "Open".into() }]);

        call_tx.send(Call::Notify { replaces: 1, summary: "Updated", urgency: 1 }).unwrap();
        assert_eq!(recv(&mut event_loop, &result_rx), "id 1");
        assert!(pump_until(&mut event_loop, || received.borrow().len() == 2));
        assert!(received.borrow()[1].replaced);
        assert_eq!(server.live.get_untracked().len(), 1);
        assert_eq!(server.live.get_untracked()[0].summary, "Updated");

        call_tx.send(Call::Notify { replaces: 99, summary: "Other", urgency: 1 }).unwrap();
        assert_eq!(recv(&mut event_loop, &result_rx), "id 2");

        server.invoke_action(2, "default");
        call_tx.send(Call::Close(2)).unwrap();
        let mut messages = Vec::new();
        for _ in 0..2 {
            messages.push(recv(&mut event_loop, &result_rx));
        }
        assert!(messages.contains(&"closed 2 3".to_owned()), "{messages:?}");
        assert!(pump_until(&mut event_loop, || closed_ids.borrow().contains(&2)));
        assert_eq!(server.live.get_untracked().len(), 1);

        server.close(1, CloseReason::Dismissed);
        assert!(server.live.get_untracked().is_empty());
    }

    #[test]
    fn a_second_server_reports_the_name_as_taken() {
        let bus = TestBus::start();
        let mut event_loop: EventLoop<'static, ()> = EventLoop::try_new().unwrap();
        let first = NotificationServer::start(&event_loop.handle(), bus.kind(), "demo").unwrap();
        let running = first.running.clone();
        assert!(pump_until(&mut event_loop, || running.get_untracked()));
        let second = NotificationServer::start(&event_loop.handle(), bus.kind(), "demo").unwrap();
        let end = Instant::now() + Duration::from_secs(1);
        while Instant::now() < end {
            event_loop.dispatch(Duration::from_millis(10), &mut ()).unwrap();
        }
        assert!(!second.running.get_untracked());
        assert!(first.running.get_untracked());
    }

    #[test]
    fn a_waiting_server_takes_the_name_over_when_the_owner_goes_away() {
        let bus = TestBus::start();
        let mut event_loop: EventLoop<'static, ()> = EventLoop::try_new().unwrap();
        let first = NotificationServer::start(&event_loop.handle(), bus.kind(), "demo").unwrap();
        let running = first.running.clone();
        assert!(pump_until(&mut event_loop, || running.get_untracked()));
        let second = NotificationServer::start(&event_loop.handle(), bus.kind(), "demo").unwrap();
        let end = Instant::now() + Duration::from_millis(300);
        while Instant::now() < end {
            event_loop.dispatch(Duration::from_millis(10), &mut ()).unwrap();
        }
        assert!(!second.running.get_untracked());
        drop(first);
        let waiting = second.running.clone();
        assert!(pump_until(&mut event_loop, || waiting.get_untracked()));
    }

    #[test]
    fn local_notices_are_published_with_ids_actions_and_callbacks() {
        let bus = TestBus::start();
        let mut event_loop: EventLoop<'static, ()> = EventLoop::try_new().unwrap();
        let server = Rc::new(NotificationServer::start(&event_loop.handle(), bus.kind(), "demo").unwrap());
        let running = server.running.clone();
        assert!(pump_until(&mut event_loop, || running.get_untracked()));
        let seen = Rc::new(RefCell::new(Vec::<String>::new()));
        let sink = seen.clone();
        server.on_notification(move |n| sink.borrow_mut().push(n.summary.clone()));
        let chosen = Rc::new(RefCell::new(Vec::<String>::new()));
        let picked = chosen.clone();
        let notifier = LocalNotifier::new(
            server.clone(),
            NotifySendNotifier::new(crate::kit::ProcessExec::shared(&event_loop.handle())),
        );
        notifier.notify(
            Notice::new("app", "Local one", "body")
                .urgency(Urgency::Critical)
                .icon("dialog-information")
                .action("default", "Open")
                .on_action(move |id| picked.borrow_mut().push(id)),
        );
        let live = server.live.get_untracked();
        assert_eq!(live.len(), 1);
        assert_eq!(
            (live[0].summary.as_str(), live[0].urgency, live[0].app_icon.as_str()),
            ("Local one", Urgency::Critical, "dialog-information")
        );
        assert_eq!(live[0].actions, [Action { id: "default".into(), text: "Open".into() }]);
        assert_eq!(*seen.borrow(), ["Local one"]);
        server.invoke_action(live[0].id, "default");
        server.invoke_action(live[0].id, "default");
        assert_eq!(*chosen.borrow(), ["default"]);
        let second = server.post_local(Notice::new("app", "Two", ""));
        assert_ne!(second, live[0].id);
        server.close(second, CloseReason::Dismissed);
        assert_eq!(server.live.get_untracked().len(), 1);
    }
}
