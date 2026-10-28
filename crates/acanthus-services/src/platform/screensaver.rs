use std::collections::HashMap;
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use acanthus_core::{Signal, batch};
use acanthus_io::IoError;
use calloop::LoopHandle;
use futures_lite::future::{self, FutureExt};
use futures_lite::{Stream, StreamExt};
use zbus::fdo::{DBusProxy, RequestNameFlags, RequestNameReply};

use zbus::{Connection, OwnedFd, interface, proxy};

use crate::platform::bridge::{Backoff, EventSink, ServiceHandle, spawn_worker};
use crate::platform::dbus::BusKind;

const SERVICE_NAME: &str = "org.freedesktop.ScreenSaver";
const OBJECT_PATHS: [&str; 2] = ["/org/freedesktop/ScreenSaver", "/ScreenSaver"];
const ACTIVITY_HOLD: Duration = Duration::from_secs(60);
const LOCK_WAIT: Duration = Duration::from_secs(4);
const RETRY_DELAY: Duration = Duration::from_secs(10);
const POLL_INTERVAL: Duration = Duration::from_millis(50);

/// System power transitions reported by logind.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PowerEvent {
    /// The system is about to sleep.
    Sleep,
    /// The system woke up.
    Wake,
}

/// Settings of the screen saver service.
#[derive(Debug, Clone)]
pub struct ScreensaverOptions {
    /// File whose existence tells that the session is locked.
    pub lock_marker: PathBuf,
    /// Name used when taking sleep inhibitors.
    pub inhibitor_name: String,
    /// How long simulated activity keeps the screen awake.
    pub activity_hold: Duration,
    /// How long to wait for the lock marker before sleeping.
    pub lock_wait: Duration,
    /// Delay before retrying a failed bus connection.
    pub retry_delay: Duration,
    /// Polling interval of the worker.
    pub poll_interval: Duration,
}

impl ScreensaverOptions {
    /// Options with the default timings and the given lock marker and inhibitor name.
    pub fn new(lock_marker: PathBuf, inhibitor_name: impl Into<String>) -> Self {
        ScreensaverOptions {
            lock_marker,
            inhibitor_name: inhibitor_name.into(),
            activity_hold: ACTIVITY_HOLD,
            lock_wait: LOCK_WAIT,
            retry_delay: RETRY_DELAY,
            poll_interval: POLL_INTERVAL,
        }
    }
}

/// Bookkeeping of inhibit requests: cookies per requester and a temporary activity hold.
#[derive(Debug, Default)]
pub struct Inhibitions {
    cookies: HashMap<u32, String>,
    next_cookie: u32,
    activity_until: Option<Instant>,
    owner: bool,
}

impl Inhibitions {
    /// Records an inhibit request from `sender` and returns its cookie.
    pub fn inhibit(&mut self, sender: &str) -> u32 {
        while self.next_cookie == 0 || self.cookies.contains_key(&self.next_cookie) {
            self.next_cookie = self.next_cookie.wrapping_add(1);
        }
        let cookie = self.next_cookie;
        self.next_cookie = self.next_cookie.wrapping_add(1);
        self.cookies.insert(cookie, sender.to_owned());
        cookie
    }

    /// Drops the request with this cookie.
    pub fn uninhibit(&mut self, cookie: u32) {
        self.cookies.remove(&cookie);
    }

    /// Holds the screen awake until `now + hold`.
    pub fn simulate_activity(&mut self, now: Instant, hold: Duration) {
        self.activity_until = Some(now + hold);
    }

    /// Drops the requests of a requester that left the bus.
    pub fn sender_vanished(&mut self, name: &str) {
        self.cookies.retain(|_, owner| owner != name);
    }

    /// Sets whether this process owns the screen saver name; losing it clears all requests.
    pub fn set_owner(&mut self, owner: bool) {
        self.owner = owner;
        if !owner {
            self.cookies.clear();
            self.activity_until = None;
        }
    }

    /// True when this process owns the name and a request or an activity hold is active.
    pub fn inhibited(&self, now: Instant) -> bool {
        self.owner && (!self.cookies.is_empty() || self.activity_until.is_some_and(|until| now < until))
    }

    /// When the activity hold ends, if one is active.
    pub fn next_deadline(&self) -> Option<Instant> {
        self.activity_until
    }

    /// Drops an activity hold that has ended.
    pub fn expire(&mut self, now: Instant) {
        if self.activity_until.is_some_and(|until| now >= until) {
            self.activity_until = None;
        }
    }
}

type Shared = Arc<Mutex<Inhibitions>>;

fn locked(shared: &Shared) -> MutexGuard<'_, Inhibitions> {
    shared.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

#[derive(Clone)]
struct ScreenSaver {
    shared: Shared,
    changed: async_channel::Sender<()>,
    activity_hold: Duration,
}

#[interface(name = "org.freedesktop.ScreenSaver")]
impl ScreenSaver {
    async fn inhibit(
        &self,
        _application_name: &str,
        _reason_for_inhibit: &str,
        #[zbus(header)] header: zbus::message::Header<'_>,
    ) -> u32 {
        let sender = header.sender().map(|sender| sender.to_string()).unwrap_or_default();
        let cookie = locked(&self.shared).inhibit(&sender);
        let _ = self.changed.send(()).await;
        cookie
    }

    async fn un_inhibit(&self, cookie: u32) {
        locked(&self.shared).uninhibit(cookie);
        let _ = self.changed.send(()).await;
    }

    async fn simulate_user_activity(&self) {
        locked(&self.shared).simulate_activity(Instant::now(), self.activity_hold);
        let _ = self.changed.send(()).await;
    }

    async fn get_active(&self) -> bool {
        false
    }
}

#[proxy(
    interface = "org.freedesktop.login1.Manager",
    default_service = "org.freedesktop.login1",
    default_path = "/org/freedesktop/login1"
)]
trait Login1 {
    fn inhibit(&self, what: &str, who: &str, why: &str, mode: &str) -> zbus::Result<OwnedFd>;

    #[zbus(signal)]
    fn prepare_for_sleep(&self, start: bool) -> zbus::Result<()>;
}

enum Event {
    Inhibited(bool),
    Power(PowerEvent),
    SleepLock(bool),
}

enum Command {
    SleepLock(bool),
}

/// Observable state of the screen saver service.
#[derive(Clone)]
pub struct ScreensaverState {
    /// True while some client inhibits the screen saver.
    pub inhibited: Signal<bool>,
    /// True while the system sleeps.
    pub sleeping: Signal<bool>,
    /// True while a sleep delay lock is held.
    pub sleep_lock_held: Signal<bool>,
}

impl ScreensaverState {
    fn new() -> Self {
        ScreensaverState {
            inhibited: Signal::new(false),
            sleeping: Signal::new(false),
            sleep_lock_held: Signal::new(false),
        }
    }

    fn apply(&self, event: &Event) {
        batch(|| match event {
            Event::Inhibited(value) => self.inhibited.set(*value),
            Event::Power(PowerEvent::Sleep) => self.sleeping.set(true),
            Event::Power(PowerEvent::Wake) => self.sleeping.set(false),
            Event::SleepLock(value) => self.sleep_lock_held.set(*value),
        });
    }
}

/// The `org.freedesktop.ScreenSaver` service: collects inhibit requests, reports power events and holds a sleep inhibitor while the session locks.
pub struct Screensaver {
    /// Its observable state.
    pub state: ScreensaverState,
    service: ServiceHandle<Command>,
}

impl Screensaver {
    /// Starts the worker on the session and system buses; `on_power` receives sleep and wake events.
    pub fn start<D: 'static>(
        handle: &LoopHandle<'static, D>,
        session: BusKind,
        system: BusKind,
        options: ScreensaverOptions,
        mut on_power: impl FnMut(PowerEvent) + 'static,
    ) -> Result<Screensaver, IoError> {
        let state = ScreensaverState::new();
        let target = state.clone();
        let service = spawn_worker(
            handle,
            "acanthus-screensaver",
            move |event| {
                target.apply(&event);
                if let Event::Power(power) = event {
                    on_power(power);
                }
            },
            move |events, commands| run(session, system, options, events, commands),
        )?;
        Ok(Screensaver { state, service })
    }

    /// Takes or releases the sleep delay lock; false when the service is gone.
    pub fn set_sleep_lock(&self, enabled: bool) -> bool {
        self.service.send(Command::SleepLock(enabled))
    }
}

async fn run(
    session: BusKind,
    system: BusKind,
    options: ScreensaverOptions,
    events: EventSink<Event>,
    commands: async_channel::Receiver<Command>,
) {
    let inhibit_server = async {
        let mut delay = None;
        loop {
            let started = Instant::now();
            match serve_screensaver(&session, &options, &events).await {
                Ok(()) => tracing::info!("screensaver service stopped"),
                Err(e) => tracing::info!("screensaver service unavailable: {e}"),
            }
            let wait = Backoff::DEFAULT.next(delay, started.elapsed());
            delay = Some(wait);
            async_io::Timer::after(wait).await;
        }
    };
    let sleep_lock = sleep_lock_loop(&system, &options, &events, commands);
    inhibit_server.or(sleep_lock).await;
}

enum Input {
    Changed,
    Vanished(String),
    Acquired,
    Lost,
    Tick,
}

async fn serve_screensaver(
    session: &BusKind,
    options: &ScreensaverOptions,
    events: &EventSink<Event>,
) -> zbus::Result<()> {
    let connection = session.connect().await?;
    let (changed_tx, changed_rx) = async_channel::unbounded();
    let shared: Shared = Arc::default();
    let service = ScreenSaver { shared: shared.clone(), changed: changed_tx, activity_hold: options.activity_hold };
    for path in OBJECT_PATHS {
        connection.object_server().at(path, service.clone()).await?;
    }
    let dbus = DBusProxy::new(&connection).await?;
    let mut owners = dbus.receive_name_owner_changed().await?;
    let mut acquired = dbus.receive_name_acquired().await?;
    let mut lost = dbus.receive_name_lost().await?;
    let reply = connection
        .request_name_with_flags(SERVICE_NAME, RequestNameFlags::ALLOW_REPLACEMENT | RequestNameFlags::REPLACE_EXISTING)
        .await?;
    locked(&shared).set_owner(matches!(reply, RequestNameReply::PrimaryOwner | RequestNameReply::AlreadyOwner));
    let mut changed_rx = std::pin::pin!(changed_rx);
    let mut published = None;
    loop {
        let now = Instant::now();
        let current = {
            let mut inhibitions = locked(&shared);
            inhibitions.expire(now);
            inhibitions.inhibited(now)
        };
        if published != Some(current) {
            published = Some(current);
            events.send(Event::Inhibited(current));
        }
        let deadline = locked(&shared).next_deadline();
        let input = async { changed_rx.next().await.map(|_| Input::Changed) }
            .or(async {
                let signal = owners.next().await?;
                let args = signal.args().ok()?;
                let name = args.name().to_string();
                (name.starts_with(':') && args.new_owner().is_none())
                    .then_some(Input::Vanished(name))
                    .or(Some(Input::Changed))
            })
            .or(async {
                let signal = acquired.next().await?;
                (signal.args().ok()?.name().as_str() == SERVICE_NAME)
                    .then_some(Input::Acquired)
                    .or(Some(Input::Changed))
            })
            .or(async {
                let signal = lost.next().await?;
                (signal.args().ok()?.name().as_str() == SERVICE_NAME).then_some(Input::Lost).or(Some(Input::Changed))
            })
            .or(async {
                match deadline {
                    Some(at) => {
                        async_io::Timer::at(at).await;
                    }
                    None => future::pending::<()>().await,
                }
                Some(Input::Tick)
            })
            .await;
        match input {
            Some(Input::Vanished(name)) => locked(&shared).sender_vanished(&name),
            Some(Input::Acquired) => locked(&shared).set_owner(true),
            Some(Input::Lost) => locked(&shared).set_owner(false),
            Some(Input::Changed | Input::Tick) => {}
            None => return Ok(()),
        }
    }
}

type SleepSignals = Pin<Box<dyn Stream<Item = bool>>>;

struct Link {
    _connection: Connection,
    signals: SleepSignals,
    inhibitor: Option<OwnedFd>,
}

async fn take_inhibitor(proxy: &Login1Proxy<'_>, who: &str) -> zbus::Result<OwnedFd> {
    proxy.inhibit("sleep", who, "Lock the screen before sleep", "delay").await
}

async fn connect_link(system: &BusKind, who: &str) -> zbus::Result<(Link, Login1Proxy<'static>)> {
    let connection = system.connect().await?;
    let proxy = Login1Proxy::new(&connection).await?;
    let signals =
        proxy.receive_prepare_for_sleep().await?.map(|signal| signal.args().map(|args| args.start).unwrap_or(false));
    let inhibitor = take_inhibitor(&proxy, who).await?;
    Ok((Link { _connection: connection, signals: Box::pin(signals), inhibitor: Some(inhibitor) }, proxy))
}

enum Wake {
    Command(Option<Command>),
    Sleep(Option<bool>),
    Tick,
}

async fn sleep_lock_loop(
    system: &BusKind,
    options: &ScreensaverOptions,
    events: &EventSink<Event>,
    commands: async_channel::Receiver<Command>,
) {
    let mut enabled = false;
    let mut link: Option<Link> = None;
    let mut proxy: Option<Login1Proxy<'static>> = None;
    let mut release_at: Option<Instant> = None;
    let mut retry_at: Option<Instant> = None;
    let mut held = false;
    loop {
        let wake = {
            let command = async { Wake::Command(commands.recv().await.ok()) };
            let signal = async {
                match link.as_mut() {
                    Some(link) => Wake::Sleep(link.signals.next().await),
                    None => future::pending().await,
                }
            };
            let tick = async {
                let at = if release_at.is_some() { Some(Instant::now() + options.poll_interval) } else { retry_at };
                match at {
                    Some(at) => {
                        async_io::Timer::at(at).await;
                    }
                    None => future::pending::<()>().await,
                }
                Wake::Tick
            };
            command.or(signal).or(tick).await
        };
        let mut failed = false;
        match wake {
            Wake::Command(None) => return,
            Wake::Command(Some(Command::SleepLock(on))) => {
                if on != enabled {
                    enabled = on;
                    release_at = None;
                    retry_at = None;
                    link = None;
                    proxy = None;
                    if on {
                        match connect_link(system, &options.inhibitor_name).await {
                            Ok((new_link, new_proxy)) => {
                                link = Some(new_link);
                                proxy = Some(new_proxy);
                            }
                            Err(e) => {
                                tracing::info!("sleep lock unavailable: {e}");
                                retry_at = Some(Instant::now() + options.retry_delay);
                            }
                        }
                    }
                }
            }
            Wake::Sleep(Some(true)) => {
                events.send(Event::Power(PowerEvent::Sleep));
                release_at = Some(Instant::now() + options.lock_wait);
            }
            Wake::Sleep(Some(false)) => {
                events.send(Event::Power(PowerEvent::Wake));
                if let (Some(active), Some(proxy)) = (link.as_mut(), proxy.as_ref())
                    && active.inhibitor.is_none()
                {
                    release_at = None;
                    match take_inhibitor(proxy, &options.inhibitor_name).await {
                        Ok(fd) => active.inhibitor = Some(fd),
                        Err(_) => failed = true,
                    }
                }
            }
            Wake::Sleep(None) => failed = true,
            Wake::Tick => {}
        }
        let now = Instant::now();
        if let Some(due) = release_at
            && (now >= due || options.lock_marker.exists())
        {
            release_at = None;
            if let Some(active) = link.as_mut() {
                active.inhibitor = None;
            }
        }
        if failed {
            link = None;
            proxy = None;
            release_at = None;
            if enabled {
                retry_at = Some(now + options.retry_delay);
            }
        }
        if let Some(due) = retry_at
            && now >= due
            && enabled
        {
            retry_at = None;
            match connect_link(system, &options.inhibitor_name).await {
                Ok((new_link, new_proxy)) => {
                    link = Some(new_link);
                    proxy = Some(new_proxy);
                }
                Err(e) => {
                    tracing::info!("sleep lock unavailable: {e}");
                    retry_at = Some(Instant::now() + options.retry_delay);
                }
            }
        }
        let now_held = link.as_ref().is_some_and(|active| active.inhibitor.is_some());
        if now_held != held {
            held = now_held;
            events.send(Event::SleepLock(held));
        }
    }
}

#[cfg(test)]
mod tests {
    use std::io::Read;
    use std::os::unix::net::UnixStream;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use calloop::EventLoop;
    use zbus::fdo;

    use super::*;
    use crate::platform::dbus::testbus::TestBus;

    fn start(hold: Duration) -> (Inhibitions, Instant, Duration) {
        let mut inhibitions = Inhibitions::default();
        inhibitions.set_owner(true);
        (inhibitions, Instant::now(), hold)
    }

    #[test]
    fn cookies_are_unique_nonzero_and_tracked_per_sender() {
        let (mut inhibitions, now, _) = start(ACTIVITY_HOLD);
        assert!(!inhibitions.inhibited(now));
        let first = inhibitions.inhibit(":1.5");
        let second = inhibitions.inhibit(":1.6");
        assert!(first != 0 && second != 0 && first != second);
        assert!(inhibitions.inhibited(now));
        inhibitions.uninhibit(first);
        assert!(inhibitions.inhibited(now));
        inhibitions.uninhibit(second);
        assert!(!inhibitions.inhibited(now));
    }

    #[test]
    fn cookie_counter_wraps_past_zero_and_used_values() {
        let mut inhibitions = Inhibitions { next_cookie: u32::MAX, owner: true, ..Inhibitions::default() };
        let last = inhibitions.inhibit(":1.1");
        let wrapped = inhibitions.inhibit(":1.1");
        assert_eq!(last, u32::MAX);
        assert_eq!(wrapped, 1);
        inhibitions.next_cookie = 1;
        assert_eq!(inhibitions.inhibit(":1.1"), 2);
    }

    #[test]
    fn a_vanished_sender_drops_only_its_cookies() {
        let (mut inhibitions, now, _) = start(ACTIVITY_HOLD);
        inhibitions.inhibit(":1.5");
        inhibitions.inhibit(":1.5");
        let kept = inhibitions.inhibit(":1.6");
        inhibitions.sender_vanished(":1.5");
        assert!(inhibitions.inhibited(now));
        inhibitions.sender_vanished(":1.6");
        assert!(!inhibitions.inhibited(now));
        inhibitions.uninhibit(kept);
    }

    #[test]
    fn simulated_activity_inhibits_until_the_hold_expires() {
        let (mut inhibitions, now, hold) = start(Duration::from_secs(60));
        inhibitions.simulate_activity(now, hold);
        assert!(inhibitions.inhibited(now + Duration::from_secs(59)));
        assert!(!inhibitions.inhibited(now + Duration::from_secs(60)));
        assert_eq!(inhibitions.next_deadline(), Some(now + hold));
        inhibitions.expire(now + Duration::from_secs(61));
        assert_eq!(inhibitions.next_deadline(), None);
    }

    #[test]
    fn without_the_name_nothing_inhibits_and_losing_it_clears_everything() {
        let mut inhibitions = Inhibitions::default();
        let now = Instant::now();
        inhibitions.inhibit(":1.1");
        assert!(!inhibitions.inhibited(now));
        inhibitions.set_owner(true);
        assert!(inhibitions.inhibited(now));
        inhibitions.simulate_activity(now, ACTIVITY_HOLD);
        inhibitions.set_owner(false);
        inhibitions.set_owner(true);
        assert!(!inhibitions.inhibited(now));
        assert_eq!(inhibitions.next_deadline(), None);
    }

    #[test]
    fn state_applies_events() {
        let state = ScreensaverState::new();
        state.apply(&Event::Inhibited(true));
        state.apply(&Event::Power(PowerEvent::Sleep));
        state.apply(&Event::SleepLock(true));
        assert!(state.inhibited.get() && state.sleeping.get() && state.sleep_lock_held.get());
        state.apply(&Event::Power(PowerEvent::Wake));
        assert!(!state.sleeping.get());
    }

    fn pump_until(event_loop: &mut EventLoop<'static, ()>, done: impl Fn() -> bool) -> bool {
        let deadline = Instant::now() + Duration::from_secs(10);
        while !done() && Instant::now() < deadline {
            event_loop.dispatch(Duration::from_millis(10), &mut ()).unwrap();
        }
        done()
    }

    fn test_options(marker: PathBuf) -> ScreensaverOptions {
        ScreensaverOptions {
            lock_marker: marker,
            inhibitor_name: "demo".to_owned(),
            activity_hold: Duration::from_millis(300),
            lock_wait: Duration::from_millis(400),
            retry_delay: Duration::from_millis(200),
            poll_interval: Duration::from_millis(20),
        }
    }

    #[test]
    fn apps_inhibit_through_the_screensaver_interface() {
        let session = TestBus::start();
        let system = TestBus::start();
        let mut event_loop: EventLoop<'static, ()> = EventLoop::try_new().unwrap();
        let marker = tempfile::tempdir().unwrap();
        let service = Screensaver::start(
            &event_loop.handle(),
            session.kind(),
            system.kind(),
            test_options(marker.path().join("locked")),
            |_| {},
        )
        .unwrap();
        let state = service.state.clone();
        let address = session.address.clone();
        let (cookie_tx, cookie_rx) = std::sync::mpsc::channel();
        let (release_tx, release_rx) = std::sync::mpsc::channel::<bool>();
        let client = std::thread::spawn(move || {
            async_io::block_on(async move {
                let connection = zbus::connection::Builder::address(address.as_str()).build().await.unwrap();
                let mut cookie = 0u32;
                for _ in 0..100 {
                    let reply = connection
                        .call_method(
                            Some(SERVICE_NAME),
                            OBJECT_PATHS[0],
                            Some(SERVICE_NAME),
                            "Inhibit",
                            &("test", "playing"),
                        )
                        .await;
                    if let Ok(reply) = reply {
                        cookie = reply.body().deserialize().unwrap();
                        break;
                    }
                    async_io::Timer::after(Duration::from_millis(50)).await;
                }
                cookie_tx.send(cookie).unwrap();
                if release_rx.recv().unwrap() {
                    connection
                        .call_method(Some(SERVICE_NAME), OBJECT_PATHS[1], Some(SERVICE_NAME), "UnInhibit", &(cookie,))
                        .await
                        .unwrap();
                    release_rx.recv().unwrap();
                }
                drop(connection);
            });
        });
        let cookie = cookie_rx.recv_timeout(Duration::from_secs(10)).unwrap();
        assert_ne!(cookie, 0);
        assert!(pump_until(&mut event_loop, || state.inhibited.get_untracked()));
        release_tx.send(true).unwrap();
        assert!(pump_until(&mut event_loop, || !state.inhibited.get_untracked()));
        release_tx.send(false).unwrap();
        client.join().unwrap();
    }

    #[test]
    fn a_client_that_disconnects_releases_its_inhibition() {
        let session = TestBus::start();
        let system = TestBus::start();
        let mut event_loop: EventLoop<'static, ()> = EventLoop::try_new().unwrap();
        let marker = tempfile::tempdir().unwrap();
        let service = Screensaver::start(
            &event_loop.handle(),
            session.kind(),
            system.kind(),
            test_options(marker.path().join("locked")),
            |_| {},
        )
        .unwrap();
        let state = service.state.clone();
        let address = session.address.clone();
        let (done_tx, done_rx) = std::sync::mpsc::channel();
        let (quit_tx, quit_rx) = std::sync::mpsc::channel::<()>();
        let client = std::thread::spawn(move || {
            async_io::block_on(async move {
                let connection = zbus::connection::Builder::address(address.as_str()).build().await.unwrap();
                for _ in 0..100 {
                    let reply = connection
                        .call_method(
                            Some(SERVICE_NAME),
                            OBJECT_PATHS[0],
                            Some(SERVICE_NAME),
                            "Inhibit",
                            &("test", "playing"),
                        )
                        .await;
                    if reply.is_ok() {
                        break;
                    }
                    async_io::Timer::after(Duration::from_millis(50)).await;
                }
                done_tx.send(()).unwrap();
                quit_rx.recv().unwrap();
                drop(connection);
            });
        });
        done_rx.recv_timeout(Duration::from_secs(10)).unwrap();
        assert!(pump_until(&mut event_loop, || state.inhibited.get_untracked()));
        quit_tx.send(()).unwrap();
        client.join().unwrap();
        assert!(pump_until(&mut event_loop, || !state.inhibited.get_untracked()));
    }

    #[test]
    fn simulated_activity_inhibits_briefly() {
        let session = TestBus::start();
        let system = TestBus::start();
        let mut event_loop: EventLoop<'static, ()> = EventLoop::try_new().unwrap();
        let marker = tempfile::tempdir().unwrap();
        let service = Screensaver::start(
            &event_loop.handle(),
            session.kind(),
            system.kind(),
            test_options(marker.path().join("locked")),
            |_| {},
        )
        .unwrap();
        let state = service.state.clone();
        let address = session.address.clone();
        let client = std::thread::spawn(move || {
            async_io::block_on(async move {
                let connection = zbus::connection::Builder::address(address.as_str()).build().await.unwrap();
                for _ in 0..100 {
                    let reply = connection
                        .call_method(
                            Some(SERVICE_NAME),
                            OBJECT_PATHS[0],
                            Some(SERVICE_NAME),
                            "SimulateUserActivity",
                            &(),
                        )
                        .await;
                    if reply.is_ok() {
                        break;
                    }
                    async_io::Timer::after(Duration::from_millis(50)).await;
                }
                async_io::Timer::after(Duration::from_millis(50)).await;
            });
        });
        assert!(pump_until(&mut event_loop, || state.inhibited.get_untracked()));
        client.join().unwrap();
        assert!(pump_until(&mut event_loop, || !state.inhibited.get_untracked()));
    }

    struct FakeLogin {
        peers: Arc<Mutex<Vec<UnixStream>>>,
        calls: Arc<AtomicUsize>,
    }

    #[interface(name = "org.freedesktop.login1.Manager")]
    impl FakeLogin {
        fn inhibit(&self, what: &str, _who: &str, _why: &str, mode: &str) -> fdo::Result<zbus::OwnedFd> {
            assert_eq!((what, mode), ("sleep", "delay"));
            let (ours, theirs) = UnixStream::pair().unwrap();
            self.peers.lock().unwrap().push(ours);
            self.calls.fetch_add(1, Ordering::SeqCst);
            Ok(zbus::OwnedFd::from(std::os::fd::OwnedFd::from(theirs)))
        }

        #[zbus(signal)]
        async fn prepare_for_sleep(emitter: &zbus::object_server::SignalEmitter<'_>, start: bool) -> zbus::Result<()>;
    }

    fn peer_closed(peer: &mut UnixStream) -> bool {
        peer.set_nonblocking(true).unwrap();
        let mut byte = [0u8; 1];
        matches!(peer.read(&mut byte), Ok(0))
    }

    fn serve_fake_login(
        address: String,
        sleeps: std::sync::mpsc::Receiver<bool>,
        ready: std::sync::mpsc::Sender<()>,
        peers: Arc<Mutex<Vec<UnixStream>>>,
        calls: Arc<AtomicUsize>,
    ) {
        std::thread::spawn(move || {
            async_io::block_on(async move {
                let connection = zbus::connection::Builder::address(address.as_str())
                    .name("org.freedesktop.login1")
                    .serve_at("/org/freedesktop/login1", FakeLogin { peers, calls })
                    .build()
                    .await
                    .unwrap();
                ready.send(()).unwrap();
                while let Ok(start) = sleeps.recv() {
                    connection
                        .emit_signal(
                            None::<&str>,
                            "/org/freedesktop/login1",
                            "org.freedesktop.login1.Manager",
                            "PrepareForSleep",
                            &(start,),
                        )
                        .await
                        .unwrap();
                }
            });
        });
    }

    #[test]
    fn sleep_lock_holds_an_inhibitor_and_releases_it_after_the_lock_marker_appears() {
        let session = TestBus::start();
        let system = TestBus::start();
        let peers = Arc::new(Mutex::new(Vec::new()));
        let calls = Arc::new(AtomicUsize::new(0));
        let (sleep_tx, sleep_rx) = std::sync::mpsc::channel();
        let (ready_tx, ready_rx) = std::sync::mpsc::channel();
        serve_fake_login(system.address.clone(), sleep_rx, ready_tx, peers.clone(), calls.clone());
        ready_rx.recv_timeout(Duration::from_secs(10)).unwrap();

        let mut event_loop: EventLoop<'static, ()> = EventLoop::try_new().unwrap();
        let dir = tempfile::tempdir().unwrap();
        let marker = dir.path().join("locked");
        let mut options = test_options(marker.clone());
        options.lock_wait = Duration::from_secs(30);
        let seen = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
        let sink = seen.clone();
        let service = Screensaver::start(&event_loop.handle(), session.kind(), system.kind(), options, move |power| {
            sink.borrow_mut().push(power)
        })
        .unwrap();
        let state = service.state.clone();
        assert!(!state.sleep_lock_held.get_untracked());
        assert!(service.set_sleep_lock(true));
        assert!(pump_until(&mut event_loop, || state.sleep_lock_held.get_untracked()));
        assert_eq!(calls.load(Ordering::SeqCst), 1);

        sleep_tx.send(true).unwrap();
        assert!(pump_until(&mut event_loop, || state.sleeping.get_untracked()));
        assert!(state.sleep_lock_held.get_untracked());
        assert!(!peer_closed(&mut peers.lock().unwrap()[0]));
        std::fs::write(&marker, "").unwrap();
        assert!(pump_until(&mut event_loop, || !state.sleep_lock_held.get_untracked()));
        assert!(peer_closed(&mut peers.lock().unwrap()[0]));

        sleep_tx.send(false).unwrap();
        assert!(
            pump_until(&mut event_loop, || !state.sleeping.get_untracked() && state.sleep_lock_held.get_untracked())
        );
        assert_eq!(calls.load(Ordering::SeqCst), 2);
        assert_eq!(*seen.borrow(), [PowerEvent::Sleep, PowerEvent::Wake]);

        assert!(service.set_sleep_lock(false));
        assert!(pump_until(&mut event_loop, || !state.sleep_lock_held.get_untracked()));
    }

    #[test]
    fn sleep_lock_gives_up_the_inhibitor_after_the_wait_without_a_marker() {
        let session = TestBus::start();
        let system = TestBus::start();
        let peers = Arc::new(Mutex::new(Vec::new()));
        let calls = Arc::new(AtomicUsize::new(0));
        let (sleep_tx, sleep_rx) = std::sync::mpsc::channel();
        let (ready_tx, ready_rx) = std::sync::mpsc::channel();
        serve_fake_login(system.address.clone(), sleep_rx, ready_tx, peers.clone(), calls.clone());
        ready_rx.recv_timeout(Duration::from_secs(10)).unwrap();

        let mut event_loop: EventLoop<'static, ()> = EventLoop::try_new().unwrap();
        let dir = tempfile::tempdir().unwrap();
        let service = Screensaver::start(
            &event_loop.handle(),
            session.kind(),
            system.kind(),
            test_options(dir.path().join("locked")),
            |_| {},
        )
        .unwrap();
        let state = service.state.clone();
        service.set_sleep_lock(true);
        assert!(pump_until(&mut event_loop, || state.sleep_lock_held.get_untracked()));
        let started = Instant::now();
        sleep_tx.send(true).unwrap();
        assert!(pump_until(&mut event_loop, || !state.sleep_lock_held.get_untracked()));
        assert!(started.elapsed() >= Duration::from_millis(350));
    }

    #[test]
    fn sleep_lock_retries_until_logind_appears() {
        let session = TestBus::start();
        let system = TestBus::start();
        let mut event_loop: EventLoop<'static, ()> = EventLoop::try_new().unwrap();
        let dir = tempfile::tempdir().unwrap();
        let service = Screensaver::start(
            &event_loop.handle(),
            session.kind(),
            system.kind(),
            test_options(dir.path().join("locked")),
            |_| {},
        )
        .unwrap();
        let state = service.state.clone();
        service.set_sleep_lock(true);
        let end = Instant::now() + Duration::from_millis(500);
        while Instant::now() < end {
            event_loop.dispatch(Duration::from_millis(20), &mut ()).unwrap();
        }
        assert!(!state.sleep_lock_held.get_untracked());

        let peers = Arc::new(Mutex::new(Vec::new()));
        let calls = Arc::new(AtomicUsize::new(0));
        let (_sleep_tx, sleep_rx) = std::sync::mpsc::channel();
        let (ready_tx, ready_rx) = std::sync::mpsc::channel();
        serve_fake_login(system.address.clone(), sleep_rx, ready_tx, peers, calls);
        ready_rx.recv_timeout(Duration::from_secs(10)).unwrap();
        assert!(pump_until(&mut event_loop, || state.sleep_lock_held.get_untracked()));
    }

    #[test]
    fn live_logind_grants_and_releases_a_sleep_inhibitor() {
        if std::env::var_os("SERP_LIVE_DBUS").is_none() {
            eprintln!("SERP_LIVE_DBUS not set, skipping the live logind check");
            return;
        }
        let session = TestBus::start();
        let mut event_loop: EventLoop<'static, ()> = EventLoop::try_new().unwrap();
        let dir = tempfile::tempdir().unwrap();
        let service = Screensaver::start(
            &event_loop.handle(),
            session.kind(),
            BusKind::System,
            test_options(dir.path().join("locked")),
            |_| {},
        )
        .unwrap();
        let state = service.state.clone();
        service.set_sleep_lock(true);
        assert!(pump_until(&mut event_loop, || state.sleep_lock_held.get_untracked()));
        service.set_sleep_lock(false);
        assert!(pump_until(&mut event_loop, || !state.sleep_lock_held.get_untracked()));
    }
}
