use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;
use std::time::Duration;

use acanthus_core::{Signal, batch};
use acanthus_io::IoError;
use calloop::channel::{Channel, Event as ChannelEvent, Sender, channel};
use calloop::{EventLoop, LoopHandle};
use calloop_wayland_source::WaylandSource;
use wayland_client::globals::{GlobalListContents, registry_queue_init};
use wayland_client::protocol::wl_registry::{self, WlRegistry};
use wayland_client::protocol::wl_seat::{self, WlSeat};
use wayland_client::{Connection, Dispatch, Proxy, QueueHandle};
use wayland_protocols::ext::idle_notify::v1::client::ext_idle_notification_v1::{self, ExtIdleNotificationV1};
use wayland_protocols::ext::idle_notify::v1::client::ext_idle_notifier_v1::{self, ExtIdleNotifierV1};

use crate::platform::bridge::{EventSink, StopFlag, ThreadHandle, spawn_thread};

const POLL_INTERVAL: Duration = Duration::from_millis(100);
const NOTIFIER_MAX_VERSION: u32 = 2;
const INPUT_ONLY_VERSION: u32 = 2;

pub fn timeout_millis(timeout: Duration) -> u32 {
    u32::try_from(timeout.as_millis()).unwrap_or(u32::MAX)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Command {
    Add { id: u64, timeout_ms: u32, respect_inhibitors: bool },
    Remove(u64),
}

enum Event {
    Ready,
    Idled(u64),
    Resumed(u64),
    Unavailable,
}

#[derive(Clone)]
pub struct IdleState {
    pub ready: Signal<bool>,
    pub available: Signal<bool>,
}

impl IdleState {
    fn new() -> Self {
        IdleState { ready: Signal::new(false), available: Signal::new(false) }
    }
}

type Monitors = Rc<RefCell<HashMap<u64, Signal<bool>>>>;

fn set_idle(monitors: &Monitors, id: u64, idle: bool) {
    let signal = monitors.borrow().get(&id).cloned();
    if let Some(signal) = signal {
        signal.set(idle);
    }
}

fn clear_all(monitors: &Monitors) {
    let signals: Vec<Signal<bool>> = monitors.borrow().values().cloned().collect();
    batch(|| {
        for signal in signals {
            signal.set(false);
        }
    });
}

pub struct Idle {
    pub state: IdleState,
    commands: Sender<Command>,
    monitors: Monitors,
    next_id: Cell<u64>,
    _thread: ThreadHandle,
}

impl Idle {
    pub fn start<D: 'static>(handle: &LoopHandle<'static, D>) -> Result<Idle, IoError> {
        let state = IdleState::new();
        let target = state.clone();
        let monitors: Monitors = Rc::default();
        let registry = monitors.clone();
        let (commands, receiver) = channel::<Command>();
        let thread = spawn_thread(
            handle,
            "serp-idle",
            move |event| match event {
                Event::Ready => {
                    batch(|| {
                        target.available.set(true);
                        target.ready.set(true);
                    });
                }
                Event::Idled(id) => set_idle(&registry, id, true),
                Event::Resumed(id) => set_idle(&registry, id, false),
                Event::Unavailable => {
                    batch(|| {
                        target.available.set(false);
                        target.ready.set(true);
                    });
                    clear_all(&registry);
                }
            },
            move |events, stop| {
                if run(&events, &stop, receiver).is_err() {
                    events.send(Event::Unavailable);
                }
            },
        )?;
        Ok(Idle { state, commands, monitors, next_id: Cell::new(0), _thread: thread })
    }

    pub fn monitor(&self, timeout: Duration, respect_inhibitors: bool) -> IdleMonitor {
        self.next_id.set(self.next_id.get() + 1);
        let id = self.next_id.get();
        let is_idle = Signal::new(false);
        self.monitors.borrow_mut().insert(id, is_idle.clone());
        let monitor = IdleMonitor {
            id,
            timeout_ms: timeout_millis(timeout),
            respect_inhibitors,
            is_idle,
            enabled: Cell::new(true),
            commands: self.commands.clone(),
            monitors: self.monitors.clone(),
        };
        monitor.send_add();
        monitor
    }
}

pub struct IdleMonitor {
    id: u64,
    timeout_ms: u32,
    respect_inhibitors: bool,
    pub is_idle: Signal<bool>,
    enabled: Cell<bool>,
    commands: Sender<Command>,
    monitors: Monitors,
}

impl IdleMonitor {
    fn send_add(&self) {
        let _ = self.commands.send(Command::Add {
            id: self.id,
            timeout_ms: self.timeout_ms,
            respect_inhibitors: self.respect_inhibitors,
        });
    }

    pub fn enabled(&self) -> bool {
        self.enabled.get()
    }

    pub fn set_enabled(&self, enabled: bool) {
        if self.enabled.replace(enabled) == enabled {
            return;
        }
        if enabled {
            self.send_add();
        } else {
            let _ = self.commands.send(Command::Remove(self.id));
            self.is_idle.set(false);
        }
    }
}

impl Drop for IdleMonitor {
    fn drop(&mut self) {
        let _ = self.commands.send(Command::Remove(self.id));
        self.monitors.borrow_mut().remove(&self.id);
    }
}

struct Worker {
    events: EventSink<Event>,
    connection: Connection,
    notifier: ExtIdleNotifierV1,
    seat: WlSeat,
    active: HashMap<u64, ExtIdleNotificationV1>,
}

impl Worker {
    fn command(&mut self, command: Command, qh: &QueueHandle<Worker>) {
        match command {
            Command::Add { id, timeout_ms, respect_inhibitors } => {
                if let Some(previous) = self.active.remove(&id) {
                    previous.destroy();
                }
                let notification = if respect_inhibitors || self.notifier.version() < INPUT_ONLY_VERSION {
                    self.notifier.get_idle_notification(timeout_ms, &self.seat, qh, id)
                } else {
                    self.notifier.get_input_idle_notification(timeout_ms, &self.seat, qh, id)
                };
                self.active.insert(id, notification);
            }
            Command::Remove(id) => {
                if let Some(notification) = self.active.remove(&id) {
                    notification.destroy();
                }
            }
        }
        let _ = self.connection.flush();
    }
}

impl Dispatch<WlRegistry, GlobalListContents> for Worker {
    fn event(
        _: &mut Self,
        _: &WlRegistry,
        _: wl_registry::Event,
        _: &GlobalListContents,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<WlSeat, ()> for Worker {
    fn event(_: &mut Self, _: &WlSeat, _: wl_seat::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {}
}

impl Dispatch<ExtIdleNotifierV1, ()> for Worker {
    fn event(
        _: &mut Self,
        _: &ExtIdleNotifierV1,
        _: ext_idle_notifier_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<ExtIdleNotificationV1, u64> for Worker {
    fn event(
        state: &mut Self,
        _: &ExtIdleNotificationV1,
        event: ext_idle_notification_v1::Event,
        id: &u64,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            ext_idle_notification_v1::Event::Idled => {
                state.events.send(Event::Idled(*id));
            }
            ext_idle_notification_v1::Event::Resumed => {
                state.events.send(Event::Resumed(*id));
            }
            _ => {}
        }
    }
}

fn run(events: &EventSink<Event>, stop: &StopFlag, commands: Channel<Command>) -> Result<(), ()> {
    let connection = Connection::connect_to_env().map_err(|_| ())?;
    let (globals, queue) = registry_queue_init::<Worker>(&connection).map_err(|_| ())?;
    let qh = queue.handle();
    let notifier: ExtIdleNotifierV1 = globals.bind(&qh, 1..=NOTIFIER_MAX_VERSION, ()).map_err(|_| ())?;
    let seat: WlSeat = globals.bind(&qh, 1..=1, ()).map_err(|_| ())?;
    let mut worker =
        Worker { events: events.clone(), connection: connection.clone(), notifier, seat, active: HashMap::new() };
    let mut event_loop: EventLoop<Worker> = EventLoop::try_new().map_err(|_| ())?;
    let handle = event_loop.handle();
    WaylandSource::new(connection.clone(), queue).insert(handle.clone()).map_err(|_| ())?;
    handle
        .insert_source(commands, move |event, _, worker| {
            if let ChannelEvent::Msg(command) = event {
                worker.command(command, &qh);
            }
        })
        .map_err(|_| ())?;
    events.send(Event::Ready);
    let signal = event_loop.get_signal();
    event_loop
        .run(POLL_INTERVAL, &mut worker, |_| {
            if stop.is_set() {
                signal.stop();
            }
        })
        .map_err(|_| ())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn registry_with(ids: &[u64]) -> Monitors {
        let monitors: Monitors = Rc::default();
        for id in ids {
            monitors.borrow_mut().insert(*id, Signal::new(false));
        }
        monitors
    }

    #[test]
    fn timeout_millis_saturates() {
        assert_eq!(timeout_millis(Duration::from_secs(120)), 120_000);
        assert_eq!(timeout_millis(Duration::from_millis(1500)), 1500);
        assert_eq!(timeout_millis(Duration::from_secs(u64::MAX / 1000)), u32::MAX);
        assert_eq!(timeout_millis(Duration::ZERO), 0);
    }

    #[test]
    fn idle_events_reach_only_the_matching_monitor() {
        let monitors = registry_with(&[1, 2]);
        set_idle(&monitors, 2, true);
        assert!(!monitors.borrow()[&1].get());
        assert!(monitors.borrow()[&2].get());
        set_idle(&monitors, 2, false);
        assert!(!monitors.borrow()[&2].get());
        set_idle(&monitors, 9, true);
    }

    #[test]
    fn clear_all_resets_every_monitor() {
        let monitors = registry_with(&[1, 2]);
        set_idle(&monitors, 1, true);
        set_idle(&monitors, 2, true);
        clear_all(&monitors);
        assert!(!monitors.borrow()[&1].get() && !monitors.borrow()[&2].get());
    }

    fn detached_monitor(commands: Sender<Command>, id: u64) -> IdleMonitor {
        let monitors = registry_with(&[id]);
        let is_idle = monitors.borrow()[&id].clone();
        IdleMonitor {
            id,
            timeout_ms: 5000,
            respect_inhibitors: true,
            is_idle,
            enabled: Cell::new(true),
            commands,
            monitors,
        }
    }

    #[test]
    fn disabling_and_enabling_sends_remove_and_add() {
        let (sender, receiver) = std::sync::mpsc::channel::<Command>();
        let (commands, channel) = channel::<Command>();
        let monitor = detached_monitor(commands, 7);
        monitor.is_idle.set(true);
        monitor.set_enabled(false);
        assert!(!monitor.is_idle.get() && !monitor.enabled());
        monitor.set_enabled(false);
        monitor.set_enabled(true);
        let mut event_loop: EventLoop<'static, ()> = EventLoop::try_new().unwrap();
        event_loop
            .handle()
            .insert_source(channel, move |event, _, _| {
                if let ChannelEvent::Msg(command) = event {
                    sender.send(command).unwrap();
                }
            })
            .unwrap();
        drop(monitor);
        event_loop.dispatch(Duration::from_millis(20), &mut ()).unwrap();
        let received: Vec<Command> = receiver.try_iter().collect();
        assert_eq!(
            received,
            [
                Command::Remove(7),
                Command::Add { id: 7, timeout_ms: 5000, respect_inhibitors: true },
                Command::Remove(7)
            ]
        );
    }

    #[test]
    fn live_compositor_accepts_a_monitor() {
        if std::env::var_os("SERP_LIVE_WAYLAND").is_none() {
            eprintln!("SERP_LIVE_WAYLAND not set, skipping the live idle check");
            return;
        }
        let mut event_loop: EventLoop<'static, ()> = EventLoop::try_new().unwrap();
        let service = Idle::start(&event_loop.handle()).unwrap();
        let monitor = service.monitor(Duration::from_secs(3600), true);
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        while !service.state.ready.get() && std::time::Instant::now() < deadline {
            event_loop.dispatch(Duration::from_millis(20), &mut ()).unwrap();
        }
        assert!(service.state.ready.get());
        assert!(service.state.available.get(), "compositor lacks ext_idle_notifier_v1");
        assert!(!monitor.is_idle.get());
    }
}
