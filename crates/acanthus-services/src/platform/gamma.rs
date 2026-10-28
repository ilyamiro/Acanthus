use std::collections::HashMap;
use std::fs::File;
use std::io::{Seek, SeekFrom, Write};
use std::time::Duration;

use acanthus_core::{Signal, batch};
use acanthus_io::IoError;
use calloop::channel::{Channel, Event as ChannelEvent, Sender, channel};
use calloop::{EventLoop, LoopHandle};
use calloop_wayland_source::WaylandSource;
use rustix::fs::{MemfdFlags, memfd_create};
use wayland_client::globals::{GlobalListContents, registry_queue_init};
use wayland_client::protocol::wl_output::{self, WlOutput};
use wayland_client::protocol::wl_registry::{self, WlRegistry};
use wayland_client::{Connection, Dispatch, Proxy, QueueHandle};
use wayland_protocols_wlr::gamma_control::v1::client::zwlr_gamma_control_manager_v1::ZwlrGammaControlManagerV1;
use wayland_protocols_wlr::gamma_control::v1::client::zwlr_gamma_control_v1::{self, ZwlrGammaControlV1};

use crate::platform::bridge::{EventSink, ThreadHandle, spawn_thread};

/// Lowest colour temperature in kelvin.
pub const TEMP_MIN: i64 = 1000;
/// Highest colour temperature in kelvin.
pub const TEMP_MAX: i64 = 10000;
/// Neutral colour temperature in kelvin, at which the screen is unchanged.
pub const DAY_TEMP: u32 = 6500;

fn helland(kelvin: f64) -> [f64; 3] {
    let t = kelvin / 100.0;
    let red = if t <= 66.0 { 255.0 } else { 329.698727446 * (t - 60.0).powf(-0.1332047592) };
    let green = if t <= 66.0 {
        99.4708025861 * t.ln() - 161.1195681661
    } else {
        288.1221695283 * (t - 60.0).powf(-0.0755148492)
    };
    let blue = if t >= 66.0 {
        255.0
    } else if t <= 19.0 {
        0.0
    } else {
        138.5177312231 * (t - 10.0).ln() - 305.0447927307
    };
    [red.clamp(0.0, 255.0), green.clamp(0.0, 255.0), blue.clamp(0.0, 255.0)]
}

/// Per-channel multipliers (red, green, blue, at most 1) that give a screen the colour temperature `kelvin`, relative to [`DAY_TEMP`].
pub fn kelvin_multipliers(kelvin: u32) -> [f64; 3] {
    let reference = helland(DAY_TEMP as f64);
    let color = helland(kelvin.clamp(TEMP_MIN as u32, TEMP_MAX as u32) as f64);
    [(color[0] / reference[0]).min(1.0), (color[1] / reference[1]).min(1.0), (color[2] / reference[2]).min(1.0)]
}

const POLL_INTERVAL: Duration = Duration::from_millis(100);
const OUTPUT_NAME_VERSION: u32 = 4;

/// Applies colour temperatures to outputs.
pub trait GammaBackend {
    /// Sets the temperature of `output`, or of every output when `None`; `None` for `kelvin` restores the neutral ramp.
    fn set(&self, output: Option<&str>, kelvin: Option<u32>);
}

/// The gamma ramp for `size` entries: three 16-bit little-endian channels scaled by `multipliers`.
pub fn ramp_bytes(size: usize, multipliers: [f64; 3]) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(size * 3 * 2);
    for channel in multipliers {
        for index in 0..size {
            let position = if size > 1 { index as f64 / (size - 1) as f64 } else { 1.0 };
            let value = (position * channel.clamp(0.0, 1.0) * 65535.0).round() as u16;
            bytes.extend_from_slice(&value.to_ne_bytes());
        }
    }
    bytes
}

/// A copy of the gamma state for tests and diagnostics.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct GammaSnapshot {
    /// Names of the outputs known.
    pub outputs: Vec<String>,
    /// Output names and the kelvin applied to each.
    pub applied: Vec<(String, u32)>,
}

enum Event {
    Snapshot(GammaSnapshot),
    Failed(String),
    Unavailable,
}

enum Command {
    Set { output: Option<String>, kelvin: Option<u32> },
}

/// Observable state of the gamma service.
#[derive(Clone)]
pub struct GammaState {
    /// True once the compositor connection is set up.
    pub ready: Signal<bool>,
    /// True when the compositor supports gamma control.
    pub available: Signal<bool>,
    /// Names of the outputs that can be adjusted.
    pub outputs: Signal<Vec<String>>,
    /// Output names and the kelvin applied to each.
    pub applied: Signal<Vec<(String, u32)>>,
}

impl GammaState {
    fn new() -> Self {
        GammaState {
            ready: Signal::new(false),
            available: Signal::new(false),
            outputs: Signal::new(Vec::new()),
            applied: Signal::new(Vec::new()),
        }
    }

    fn apply(&self, snapshot: GammaSnapshot) {
        batch(|| {
            self.available.set(true);
            if self.outputs.with_untracked(|current| *current != snapshot.outputs) {
                self.outputs.set(snapshot.outputs);
            }
            if self.applied.with_untracked(|current| *current != snapshot.applied) {
                self.applied.set(snapshot.applied);
            }
            self.ready.set(true);
        });
    }

    fn lost(&self) {
        batch(|| {
            self.available.set(false);
            self.outputs.set(Vec::new());
            self.applied.set(Vec::new());
            self.ready.set(true);
        });
    }
}

/// Colour temperature control through `wlr-gamma-control`.
pub struct Gamma {
    /// Its observable state.
    pub state: GammaState,
    commands: Sender<Command>,
    _thread: ThreadHandle,
}

impl Gamma {
    /// Connects to the compositor on a helper thread; `on_failed` receives a message when control is lost.
    pub fn start<D: 'static>(
        handle: &LoopHandle<'static, D>,
        mut on_failed: impl FnMut(String) + 'static,
    ) -> Result<Gamma, IoError> {
        let state = GammaState::new();
        let target = state.clone();
        let (commands, receiver) = channel::<Command>();
        let thread = spawn_thread(
            handle,
            "acanthus-gamma",
            move |event| match event {
                Event::Snapshot(snapshot) => target.apply(snapshot),
                Event::Failed(name) => on_failed(name),
                Event::Unavailable => target.lost(),
            },
            move |events, stop| {
                if run(&events, &stop, receiver).is_err() {
                    events.send(Event::Unavailable);
                }
            },
        )?;
        Ok(Gamma { state, commands, _thread: thread })
    }
}

impl GammaBackend for Gamma {
    fn set(&self, output: Option<&str>, kelvin: Option<u32>) {
        let _ = self.commands.send(Command::Set { output: output.map(str::to_owned), kelvin });
    }
}

struct Slot {
    registry_name: u32,
    output: WlOutput,
    name: Option<String>,
    control: Option<ZwlrGammaControlV1>,
    size: u32,
    applied: Option<u32>,
    failed: bool,
}

struct Worker {
    events: EventSink<Event>,
    manager: ZwlrGammaControlManagerV1,
    qh: QueueHandle<Worker>,
    slots: Vec<Slot>,
    global: Option<u32>,
    targets: HashMap<String, u32>,
    last: Option<GammaSnapshot>,
}

impl Worker {
    fn desired(&self, name: &str) -> Option<u32> {
        self.targets.get(name).copied().or(self.global)
    }

    fn command(&mut self, command: Command) {
        let Command::Set { output, kelvin } = command;
        match (output, kelvin) {
            (None, kelvin) => self.global = kelvin,
            (Some(name), Some(kelvin)) => {
                self.targets.insert(name, kelvin);
            }
            (Some(name), None) => {
                self.targets.remove(&name);
            }
        }
        for slot in &mut self.slots {
            slot.failed = false;
        }
        self.reconcile();
    }

    fn reconcile(&mut self) {
        for index in 0..self.slots.len() {
            let Some(name) = self.slots[index].name.clone() else { continue };
            let desired = self.desired(&name);
            let slot = &mut self.slots[index];
            match desired {
                None => {
                    if let Some(control) = slot.control.take() {
                        control.destroy();
                    }
                    slot.size = 0;
                    slot.applied = None;
                }
                Some(kelvin) => {
                    if slot.failed {
                        continue;
                    }
                    if slot.control.is_none() {
                        slot.control = Some(self.manager.get_gamma_control(&slot.output, &self.qh, slot.registry_name));
                    }
                    if slot.size > 0
                        && slot.applied != Some(kelvin)
                        && let Some(control) = &slot.control
                        && write_ramp(control, slot.size as usize, kelvin).is_ok()
                    {
                        slot.applied = Some(kelvin);
                    }
                }
            }
        }
        self.publish();
    }

    fn publish(&mut self) {
        let snapshot = GammaSnapshot {
            outputs: self.slots.iter().filter_map(|slot| slot.name.clone()).collect(),
            applied: self.slots.iter().filter_map(|slot| Some((slot.name.clone()?, slot.applied?))).collect(),
        };
        if self.last.as_ref() != Some(&snapshot) {
            self.last = Some(snapshot.clone());
            self.events.send(Event::Snapshot(snapshot));
        }
    }
}

fn write_ramp(control: &ZwlrGammaControlV1, size: usize, kelvin: u32) -> std::io::Result<()> {
    let bytes = ramp_bytes(size, kelvin_multipliers(kelvin));
    let fd = memfd_create("acanthus-gamma", MemfdFlags::CLOEXEC).map_err(std::io::Error::from)?;
    let mut file = File::from(fd);
    file.write_all(&bytes)?;
    file.seek(SeekFrom::Start(0))?;
    control.set_gamma(std::os::fd::AsFd::as_fd(&file));
    Ok(())
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
            wl_registry::Event::Global { name, interface, version } if interface == WlOutput::interface().name => {
                let output = registry.bind::<WlOutput, _, _>(name, version.min(OUTPUT_NAME_VERSION), qh, name);
                state.slots.push(Slot {
                    registry_name: name,
                    output,
                    name: None,
                    control: None,
                    size: 0,
                    applied: None,
                    failed: false,
                });
            }
            wl_registry::Event::GlobalRemove { name } => {
                if let Some(position) = state.slots.iter().position(|slot| slot.registry_name == name) {
                    let slot = state.slots.remove(position);
                    if let Some(control) = slot.control {
                        control.destroy();
                    }
                    state.publish();
                }
            }
            _ => {}
        }
    }
}

impl Dispatch<WlOutput, u32> for Worker {
    fn event(
        state: &mut Self,
        _: &WlOutput,
        event: wl_output::Event,
        registry_name: &u32,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let wl_output::Event::Name { name } = event {
            if let Some(slot) = state.slots.iter_mut().find(|slot| slot.registry_name == *registry_name) {
                slot.name = Some(name);
            }
            state.reconcile();
        }
    }
}

impl Dispatch<ZwlrGammaControlManagerV1, ()> for Worker {
    fn event(
        _: &mut Self,
        _: &ZwlrGammaControlManagerV1,
        _: <ZwlrGammaControlManagerV1 as Proxy>::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<ZwlrGammaControlV1, u32> for Worker {
    fn event(
        state: &mut Self,
        _: &ZwlrGammaControlV1,
        event: zwlr_gamma_control_v1::Event,
        registry_name: &u32,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        let Some(slot) = state.slots.iter_mut().find(|slot| slot.registry_name == *registry_name) else { return };
        match event {
            zwlr_gamma_control_v1::Event::GammaSize { size } => {
                slot.size = size;
                slot.applied = None;
                state.reconcile();
            }
            zwlr_gamma_control_v1::Event::Failed => {
                slot.failed = true;
                slot.applied = None;
                slot.size = 0;
                if let Some(control) = slot.control.take() {
                    control.destroy();
                }
                let name = slot.name.clone().unwrap_or_default();
                state.events.send(Event::Failed(name));
                state.publish();
            }
            _ => {}
        }
    }
}

fn run(
    events: &EventSink<Event>,
    stop: &crate::platform::bridge::StopFlag,
    commands: Channel<Command>,
) -> Result<(), ()> {
    let connection = Connection::connect_to_env().map_err(|_| ())?;
    let (globals, queue) = registry_queue_init::<Worker>(&connection).map_err(|_| ())?;
    let qh = queue.handle();
    let manager: ZwlrGammaControlManagerV1 = globals.bind(&qh, 1..=1, ()).map_err(|_| ())?;
    let mut worker = Worker {
        events: events.clone(),
        manager,
        qh: qh.clone(),
        slots: Vec::new(),
        global: None,
        targets: HashMap::new(),
        last: None,
    };
    let registry = globals.registry();
    globals.contents().with_list(|list| {
        for global in list {
            if global.interface == WlOutput::interface().name {
                let output = registry.bind::<WlOutput, _, _>(
                    global.name,
                    global.version.min(OUTPUT_NAME_VERSION),
                    &qh,
                    global.name,
                );
                worker.slots.push(Slot {
                    registry_name: global.name,
                    output,
                    name: None,
                    control: None,
                    size: 0,
                    applied: None,
                    failed: false,
                });
            }
        }
    });
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
    worker.publish();
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

    fn words(bytes: &[u8]) -> Vec<u16> {
        bytes.chunks(2).map(|pair| u16::from_ne_bytes([pair[0], pair[1]])).collect()
    }

    #[test]
    fn neutral_ramp_is_linear_for_all_channels() {
        let ramp = words(&ramp_bytes(5, [1.0, 1.0, 1.0]));
        assert_eq!(ramp.len(), 15);
        for channel in ramp.chunks(5) {
            assert_eq!(channel, [0, 16384, 32768, 49151, 65535]);
        }
    }

    #[test]
    fn ramp_scales_each_channel_and_clamps() {
        let ramp = words(&ramp_bytes(3, [1.0, 0.5, 2.0]));
        assert_eq!(&ramp[0..3], [0, 32768, 65535]);
        assert_eq!(&ramp[3..6], [0, 16384, 32768]);
        assert_eq!(&ramp[6..9], [0, 32768, 65535]);
    }

    #[test]
    fn single_entry_ramp_is_full_scale() {
        assert_eq!(words(&ramp_bytes(1, [0.5, 1.0, 0.0])), [32768, 65535, 0]);
    }

    #[test]
    fn live_compositor_accepts_a_temperature() {
        if std::env::var_os("SERP_LIVE_GAMMA").is_none() {
            eprintln!("SERP_LIVE_GAMMA not set, skipping the live gamma check");
            return;
        }
        let mut event_loop: EventLoop<'static, ()> = EventLoop::try_new().unwrap();
        let failed = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
        let sink = failed.clone();
        let gamma = Gamma::start(&event_loop.handle(), move |name| sink.borrow_mut().push(name)).unwrap();
        let pump = |event_loop: &mut EventLoop<'static, ()>, ms: u64| {
            let end = std::time::Instant::now() + Duration::from_millis(ms);
            while std::time::Instant::now() < end {
                event_loop.dispatch(Duration::from_millis(20), &mut ()).unwrap();
            }
        };
        pump(&mut event_loop, 500);
        assert!(gamma.state.available.get(), "compositor lacks zwlr_gamma_control_manager_v1");
        eprintln!("outputs {:?}", gamma.state.outputs.get());
        gamma.set(None, Some(5500));
        pump(&mut event_loop, 1500);
        eprintln!("applied {:?} failed {:?}", gamma.state.applied.get(), failed.borrow());
        gamma.set(None, None);
        pump(&mut event_loop, 500);
        eprintln!("after reset applied {:?}", gamma.state.applied.get());
        assert!(gamma.state.applied.get().is_empty());
    }
}
