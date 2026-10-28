use std::time::Duration;

use acanthus_core::{Signal, batch};
use acanthus_io::IoError;
use calloop::channel::{Channel, Event as ChannelEvent, Sender, channel};
use calloop::{EventLoop, LoopHandle};
use calloop_wayland_source::WaylandSource;
use wayland_client::globals::{GlobalListContents, registry_queue_init};
use wayland_client::protocol::wl_callback::{self, WlCallback};
use wayland_client::protocol::wl_output::{self, WlOutput};
use wayland_client::protocol::wl_registry::{self, WlRegistry};
use wayland_client::protocol::wl_seat::{self, WlSeat};
use wayland_client::{Connection, Dispatch, Proxy, QueueHandle};
use wayland_protocols_wlr::foreign_toplevel::v1::client::zwlr_foreign_toplevel_handle_v1::{
    self, ZwlrForeignToplevelHandleV1,
};
use wayland_protocols_wlr::foreign_toplevel::v1::client::zwlr_foreign_toplevel_manager_v1::{
    self, ZwlrForeignToplevelManagerV1,
};

use crate::platform::bridge::{EventSink, ThreadHandle, spawn_thread};

const STATE_MAXIMIZED: u32 = 0;
const STATE_MINIMIZED: u32 = 1;
const STATE_ACTIVATED: u32 = 2;
const STATE_FULLSCREEN: u32 = 3;
const POLL_INTERVAL: Duration = Duration::from_millis(100);
const OUTPUT_NAME_VERSION: u32 = 4;
const MANAGER_MAX_VERSION: u32 = 3;

/// State flags of a toplevel window.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ToplevelFlags {
    /// The window is maximized.
    pub maximized: bool,
    /// The window is minimized.
    pub minimized: bool,
    /// The window has focus.
    pub activated: bool,
    /// The window is fullscreen.
    pub fullscreen: bool,
}

/// Decodes the state array of the foreign-toplevel protocol.
pub fn state_flags(bytes: &[u8]) -> ToplevelFlags {
    let mut flags = ToplevelFlags::default();
    for chunk in bytes.as_chunks::<4>().0 {
        match u32::from_ne_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]) {
            STATE_MAXIMIZED => flags.maximized = true,
            STATE_MINIMIZED => flags.minimized = true,
            STATE_ACTIVATED => flags.activated = true,
            STATE_FULLSCREEN => flags.fullscreen = true,
            _ => {}
        }
    }
    flags
}

/// A window reported by the compositor.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Toplevel {
    /// Identifier valid while the service runs.
    pub id: u64,
    /// Application id.
    pub app_id: String,
    /// Title.
    pub title: String,
    /// Names of the outputs it is on.
    pub outputs: Vec<String>,
    /// True when it has focus.
    pub activated: bool,
    /// True when it is minimized.
    pub minimized: bool,
    /// True when it is maximized.
    pub maximized: bool,
    /// True when it is fullscreen.
    pub fullscreen: bool,
    /// The window it is a dialog of.
    pub parent: Option<u64>,
}

impl Toplevel {
    /// The first output it is on, empty when none.
    pub fn output(&self) -> &str {
        self.outputs.first().map(String::as_str).unwrap_or("")
    }
}

/// A request to change a window.
#[derive(Debug, Clone, PartialEq)]
pub enum ToplevelCommand {
    /// Gives the window focus.
    Activate(u64),
    /// Closes the window.
    Close(u64),
    /// Minimizes or restores the window.
    SetMinimized(u64, bool),
    /// Maximizes or restores the window.
    SetMaximized(u64, bool),
    /// Makes the window fullscreen or restores it.
    SetFullscreen(u64, bool),
}

/// All windows at one moment.
#[derive(Debug, Clone, PartialEq)]
pub struct ToplevelSnapshot {
    /// The windows.
    pub toplevels: Vec<Toplevel>,
}

enum Event {
    Snapshot(ToplevelSnapshot),
    Unavailable,
}

/// Observable state of the toplevel service.
#[derive(Clone)]
pub struct ToplevelsState {
    /// True once the first list arrived.
    pub ready: Signal<bool>,
    /// True when the compositor supports toplevel management.
    pub available: Signal<bool>,
    /// All windows.
    pub toplevels: Signal<Vec<Toplevel>>,
    /// Id of the focused window.
    pub active: Signal<Option<u64>>,
}

impl Default for ToplevelsState {
    fn default() -> Self {
        ToplevelsState::new()
    }
}

impl ToplevelsState {
    /// State with no windows.
    pub fn new() -> Self {
        ToplevelsState {
            ready: Signal::new(false),
            available: Signal::new(false),
            toplevels: Signal::new(Vec::new()),
            active: Signal::new(None),
        }
    }

    /// Copies a snapshot into the signals in one batch.
    pub fn apply(&self, snapshot: &ToplevelSnapshot) {
        batch(|| {
            self.available.set(true);
            if self.toplevels.with_untracked(|current| *current != snapshot.toplevels) {
                self.toplevels.set(snapshot.toplevels.clone());
            }
            self.active.set(snapshot.toplevels.iter().find(|t| t.activated).map(|t| t.id));
            self.ready.set(true);
        });
    }

    fn lost(&self) {
        batch(|| {
            self.available.set(false);
            self.toplevels.set(Vec::new());
            self.active.set(None);
            self.ready.set(true);
        });
    }

    /// The window with this id.
    pub fn toplevel(&self, id: u64) -> Option<Toplevel> {
        self.toplevels.with_untracked(|list| list.iter().find(|t| t.id == id).cloned())
    }

    /// The focused window.
    pub fn active_toplevel(&self) -> Option<Toplevel> {
        self.active.with_untracked(|id| id.and_then(|id| self.toplevel(id)))
    }
}

/// Windows of all applications through the foreign-toplevel protocol, with control over them.
pub struct Toplevels {
    /// Its observable state.
    pub state: ToplevelsState,
    commands: Sender<ToplevelCommand>,
    _thread: ThreadHandle,
}

impl Toplevels {
    /// Connects to the compositor on a helper thread.
    pub fn start<D: 'static>(handle: &LoopHandle<'static, D>) -> Result<Toplevels, IoError> {
        let state = ToplevelsState::new();
        let target = state.clone();
        let (commands, receiver) = channel::<ToplevelCommand>();
        let thread = spawn_thread(
            handle,
            "acanthus-toplevels",
            move |event| match event {
                Event::Snapshot(snapshot) => target.apply(&snapshot),
                Event::Unavailable => target.lost(),
            },
            move |events, stop| {
                if run(&events, &stop, receiver).is_err() {
                    events.send(Event::Unavailable);
                }
            },
        )?;
        Ok(Toplevels { state, commands, _thread: thread })
    }

    /// Sends a command; false when the service is gone.
    pub fn command(&self, command: ToplevelCommand) -> bool {
        self.commands.send(command).is_ok()
    }

    /// Gives the window focus; false when the service is gone.
    pub fn activate(&self, id: u64) -> bool {
        self.command(ToplevelCommand::Activate(id))
    }

    /// Closes the window; false when the service is gone.
    pub fn close(&self, id: u64) -> bool {
        self.command(ToplevelCommand::Close(id))
    }

    /// Minimizes or restores the window; false when the service is gone.
    pub fn set_minimized(&self, id: u64, minimized: bool) -> bool {
        self.command(ToplevelCommand::SetMinimized(id, minimized))
    }

    /// Maximizes or restores the window; false when the service is gone.
    pub fn set_maximized(&self, id: u64, maximized: bool) -> bool {
        self.command(ToplevelCommand::SetMaximized(id, maximized))
    }

    /// Makes the window fullscreen or restores it; false when the service is gone.
    pub fn set_fullscreen(&self, id: u64, fullscreen: bool) -> bool {
        self.command(ToplevelCommand::SetFullscreen(id, fullscreen))
    }
}

struct Pending {
    id: u64,
    handle: ZwlrForeignToplevelHandleV1,
    app_id: String,
    title: String,
    outputs: Vec<WlOutput>,
    flags: ToplevelFlags,
    parent: Option<ZwlrForeignToplevelHandleV1>,
    announced: bool,
}

struct Worker {
    events: EventSink<Event>,
    connection: Connection,
    seat: Option<WlSeat>,
    outputs: Vec<(WlOutput, Option<String>)>,
    entries: Vec<Pending>,
    next_id: u64,
    ready: bool,
    finished: bool,
    last: Option<Vec<Toplevel>>,
}

impl Worker {
    fn output_name(&self, output: &WlOutput) -> Option<String> {
        self.outputs.iter().find(|(known, _)| known == output).and_then(|(_, name)| name.clone())
    }

    fn snapshot(&self) -> Vec<Toplevel> {
        self.entries
            .iter()
            .filter(|entry| entry.announced)
            .map(|entry| Toplevel {
                id: entry.id,
                app_id: entry.app_id.clone(),
                title: entry.title.clone(),
                outputs: entry.outputs.iter().filter_map(|output| self.output_name(output)).collect(),
                activated: entry.flags.activated,
                minimized: entry.flags.minimized,
                maximized: entry.flags.maximized,
                fullscreen: entry.flags.fullscreen,
                parent: entry.parent.as_ref().and_then(|parent| {
                    self.entries.iter().find(|other| other.announced && other.handle == *parent).map(|other| other.id)
                }),
            })
            .collect()
    }

    fn publish(&mut self) {
        if !self.ready {
            return;
        }
        let toplevels = self.snapshot();
        if self.last.as_ref() != Some(&toplevels) {
            self.last = Some(toplevels.clone());
            self.events.send(Event::Snapshot(ToplevelSnapshot { toplevels }));
        }
    }

    fn entry_mut(&mut self, handle: &ZwlrForeignToplevelHandleV1) -> Option<&mut Pending> {
        self.entries.iter_mut().find(|entry| entry.handle == *handle)
    }

    fn command(&mut self, command: ToplevelCommand) {
        let id = match command {
            ToplevelCommand::Activate(id)
            | ToplevelCommand::Close(id)
            | ToplevelCommand::SetMinimized(id, _)
            | ToplevelCommand::SetMaximized(id, _)
            | ToplevelCommand::SetFullscreen(id, _) => id,
        };
        let Some(entry) = self.entries.iter().find(|entry| entry.id == id) else {
            return;
        };
        let handle = &entry.handle;
        match command {
            ToplevelCommand::Activate(_) => {
                if let Some(seat) = &self.seat {
                    handle.activate(seat);
                }
            }
            ToplevelCommand::Close(_) => handle.close(),
            ToplevelCommand::SetMinimized(_, true) => handle.set_minimized(),
            ToplevelCommand::SetMinimized(_, false) => handle.unset_minimized(),
            ToplevelCommand::SetMaximized(_, true) => handle.set_maximized(),
            ToplevelCommand::SetMaximized(_, false) => handle.unset_maximized(),
            ToplevelCommand::SetFullscreen(_, true) => {
                if handle.version() >= 2 {
                    handle.set_fullscreen(None);
                }
            }
            ToplevelCommand::SetFullscreen(_, false) => {
                if handle.version() >= 2 {
                    handle.unset_fullscreen();
                }
            }
        }
        let _ = self.connection.flush();
    }
}

impl Dispatch<WlRegistry, GlobalListContents> for Worker {
    fn event(
        state: &mut Self,
        registry: &WlRegistry,
        event: wl_registry::Event,
        _: &GlobalListContents,
        _: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        match event {
            wl_registry::Event::Global { name, interface, version } => {
                if interface == WlOutput::interface().name {
                    let output = registry.bind::<WlOutput, _, _>(name, version.min(OUTPUT_NAME_VERSION), qh, ());
                    state.outputs.push((output, None));
                }
            }
            wl_registry::Event::GlobalRemove { .. } => {}
            _ => {}
        }
    }
}

impl Dispatch<WlOutput, ()> for Worker {
    fn event(
        state: &mut Self,
        output: &WlOutput,
        event: wl_output::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let wl_output::Event::Name { name } = event {
            if let Some(slot) = state.outputs.iter_mut().find(|(known, _)| known == output) {
                slot.1 = Some(name);
            }
            state.publish();
        }
    }
}

impl Dispatch<WlSeat, ()> for Worker {
    fn event(_: &mut Self, _: &WlSeat, _: wl_seat::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {}
}

impl Dispatch<WlCallback, ()> for Worker {
    fn event(
        state: &mut Self,
        _: &WlCallback,
        event: wl_callback::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let wl_callback::Event::Done { .. } = event {
            state.ready = true;
            state.publish();
        }
    }
}

impl Dispatch<ZwlrForeignToplevelManagerV1, ()> for Worker {
    fn event(
        state: &mut Self,
        _: &ZwlrForeignToplevelManagerV1,
        event: zwlr_foreign_toplevel_manager_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            zwlr_foreign_toplevel_manager_v1::Event::Toplevel { toplevel } => {
                state.next_id += 1;
                state.entries.push(Pending {
                    id: state.next_id,
                    handle: toplevel,
                    app_id: String::new(),
                    title: String::new(),
                    outputs: Vec::new(),
                    flags: ToplevelFlags::default(),
                    parent: None,
                    announced: false,
                });
            }
            zwlr_foreign_toplevel_manager_v1::Event::Finished => state.finished = true,
            _ => {}
        }
    }

    wayland_client::event_created_child!(Worker, ZwlrForeignToplevelManagerV1, [
        zwlr_foreign_toplevel_manager_v1::EVT_TOPLEVEL_OPCODE => (ZwlrForeignToplevelHandleV1, ()),
    ]);
}

impl Dispatch<ZwlrForeignToplevelHandleV1, ()> for Worker {
    fn event(
        state: &mut Self,
        handle: &ZwlrForeignToplevelHandleV1,
        event: zwlr_foreign_toplevel_handle_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        use zwlr_foreign_toplevel_handle_v1::Event as E;
        match event {
            E::Title { title } => {
                if let Some(entry) = state.entry_mut(handle) {
                    entry.title = title;
                }
            }
            E::AppId { app_id } => {
                if let Some(entry) = state.entry_mut(handle) {
                    entry.app_id = app_id;
                }
            }
            E::OutputEnter { output } => {
                if let Some(entry) = state.entry_mut(handle)
                    && !entry.outputs.contains(&output)
                {
                    entry.outputs.push(output);
                }
            }
            E::OutputLeave { output } => {
                if let Some(entry) = state.entry_mut(handle) {
                    entry.outputs.retain(|known| *known != output);
                }
            }
            E::State { state: bytes } => {
                if let Some(entry) = state.entry_mut(handle) {
                    entry.flags = state_flags(&bytes);
                }
            }
            E::Parent { parent } => {
                if let Some(entry) = state.entry_mut(handle) {
                    entry.parent = parent;
                }
            }
            E::Done => {
                if let Some(entry) = state.entry_mut(handle) {
                    entry.announced = true;
                }
                state.publish();
            }
            E::Closed => {
                state.entries.retain(|entry| entry.handle != *handle);
                handle.destroy();
                state.publish();
            }
            _ => {}
        }
    }
}

fn run(
    events: &EventSink<Event>,
    stop: &crate::platform::bridge::StopFlag,
    commands: Channel<ToplevelCommand>,
) -> Result<(), ()> {
    let connection = Connection::connect_to_env().map_err(|_| ())?;
    let (globals, queue) = registry_queue_init::<Worker>(&connection).map_err(|_| ())?;
    let qh = queue.handle();
    let _manager: ZwlrForeignToplevelManagerV1 = globals.bind(&qh, 1..=MANAGER_MAX_VERSION, ()).map_err(|_| ())?;
    let mut worker = Worker {
        events: events.clone(),
        connection: connection.clone(),
        seat: None,
        outputs: Vec::new(),
        entries: Vec::new(),
        next_id: 0,
        ready: false,
        finished: false,
        last: None,
    };
    let registry = globals.registry();
    globals.contents().with_list(|list| {
        for global in list {
            if global.interface == WlOutput::interface().name {
                let output =
                    registry.bind::<WlOutput, _, _>(global.name, global.version.min(OUTPUT_NAME_VERSION), &qh, ());
                worker.outputs.push((output, None));
            } else if global.interface == WlSeat::interface().name && worker.seat.is_none() {
                worker.seat = Some(registry.bind::<WlSeat, _, _>(global.name, global.version.min(1), &qh, ()));
            }
        }
    });
    connection.display().sync(&qh, ());
    let mut event_loop: EventLoop<Worker> = EventLoop::try_new().map_err(|_| ())?;
    let handle = event_loop.handle();
    WaylandSource::new(connection.clone(), queue).insert(handle.clone()).map_err(|_| ())?;
    handle
        .insert_source(commands, |event, _, worker| {
            if let ChannelEvent::Msg(command) = event {
                worker.command(command);
            }
        })
        .map_err(|_| ())?;
    let signal = event_loop.get_signal();
    let result = event_loop.run(POLL_INTERVAL, &mut worker, |worker| {
        if stop.is_set() || worker.finished {
            signal.stop();
        }
    });
    if worker.finished {
        return Err(());
    }
    result.map_err(|_| ())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bytes(values: &[u32]) -> Vec<u8> {
        values.iter().flat_map(|value| value.to_ne_bytes()).collect()
    }

    fn toplevel(id: u64, activated: bool) -> Toplevel {
        Toplevel { id, app_id: format!("app{id}"), title: format!("title{id}"), activated, ..Toplevel::default() }
    }

    #[test]
    fn state_flags_decode_every_known_state() {
        assert_eq!(state_flags(&bytes(&[])), ToplevelFlags::default());
        let all = state_flags(&bytes(&[0, 1, 2, 3]));
        assert!(all.maximized && all.minimized && all.activated && all.fullscreen);
        let only = state_flags(&bytes(&[2]));
        assert_eq!(only, ToplevelFlags { activated: true, ..ToplevelFlags::default() });
    }

    #[test]
    fn state_flags_ignore_unknown_values_and_trailing_bytes() {
        let mut data = bytes(&[9, 1]);
        data.push(7);
        assert_eq!(state_flags(&data), ToplevelFlags { minimized: true, ..ToplevelFlags::default() });
    }

    #[test]
    fn apply_tracks_the_active_toplevel() {
        let state = ToplevelsState::new();
        assert!(!state.ready.get());
        state.apply(&ToplevelSnapshot { toplevels: vec![toplevel(1, false), toplevel(2, true)] });
        assert!(state.ready.get() && state.available.get());
        assert_eq!(state.active.get(), Some(2));
        assert_eq!(state.active_toplevel().unwrap().app_id, "app2");
        state.apply(&ToplevelSnapshot { toplevels: vec![toplevel(1, false)] });
        assert_eq!(state.active.get(), None);
        assert!(state.active_toplevel().is_none());
        assert_eq!(state.toplevel(1).unwrap().title, "title1");
        assert!(state.toplevel(2).is_none());
    }

    #[test]
    fn lost_clears_everything_but_stays_ready() {
        let state = ToplevelsState::new();
        state.apply(&ToplevelSnapshot { toplevels: vec![toplevel(1, true)] });
        state.lost();
        assert!(state.ready.get() && !state.available.get());
        assert!(state.toplevels.get().is_empty());
        assert_eq!(state.active.get(), None);
    }

    #[test]
    fn output_is_the_first_output_or_empty() {
        let mut item = toplevel(1, false);
        assert_eq!(item.output(), "");
        item.outputs = vec!["DP-1".into(), "HDMI-A-1".into()];
        assert_eq!(item.output(), "DP-1");
    }

    #[test]
    fn live_compositor_lists_toplevels() {
        if std::env::var_os("SERP_LIVE_WAYLAND").is_none() {
            eprintln!("SERP_LIVE_WAYLAND not set, skipping the live toplevel check");
            return;
        }
        let mut event_loop: EventLoop<'static, ()> = EventLoop::try_new().unwrap();
        let service = Toplevels::start(&event_loop.handle()).unwrap();
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        while !service.state.ready.get() && std::time::Instant::now() < deadline {
            event_loop.dispatch(Duration::from_millis(20), &mut ()).unwrap();
        }
        assert!(service.state.ready.get());
        assert!(service.state.available.get(), "compositor lacks zwlr_foreign_toplevel_management_v1");
        for item in service.state.toplevels.get() {
            eprintln!("{item:?}");
        }
    }
}
