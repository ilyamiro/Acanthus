use std::collections::HashMap;
use std::ptr::NonNull;
use std::time::{Duration, Instant};

use crate::dnd::DragOutcome;
use acanthus_core::{Signal, Vec2};
use acanthus_render::{Gpu, GpuError};
use acanthus_scene::{
    CursorShape, DragData, DropActions, Fonts, ImageStore, Key, KeyEvent, Modifiers, MouseButton, Scene, ScrollDelta,
};
use raw_window_handle::{RawDisplayHandle, RawWindowHandle, WaylandDisplayHandle, WaylandWindowHandle};
use smithay_client_toolkit::compositor::{CompositorHandler, CompositorState};
use smithay_client_toolkit::output::{OutputHandler, OutputState};
use smithay_client_toolkit::reexports::calloop::timer::{TimeoutAction, Timer};
use smithay_client_toolkit::reexports::calloop::{self, EventLoop};
use smithay_client_toolkit::reexports::calloop_wayland_source::WaylandSource;
use smithay_client_toolkit::reexports::protocols::wp::cursor_shape::v1::client::wp_cursor_shape_device_v1::{
    Shape, WpCursorShapeDeviceV1,
};
use smithay_client_toolkit::registry::{ProvidesRegistryState, RegistryState};
use smithay_client_toolkit::seat::keyboard::{
    KeyEvent as RawKeyEvent, KeyboardHandler, Keysym, Modifiers as RawModifiers,
};
use smithay_client_toolkit::seat::pointer::cursor_shape::CursorShapeManager;
use smithay_client_toolkit::seat::pointer::{PointerEvent, PointerEventKind, PointerHandler};
use smithay_client_toolkit::seat::{Capability, SeatHandler, SeatState};
use smithay_client_toolkit::session_lock::{
    SessionLock, SessionLockHandler, SessionLockState, SessionLockSurface, SessionLockSurfaceConfigure,
};
use smithay_client_toolkit::shell::WaylandSurface;
use smithay_client_toolkit::shell::wlr_layer::{
    Anchor, KeyboardInteractivity, Layer, LayerShell, LayerShellHandler, LayerSurface, LayerSurfaceConfigure,
};
use smithay_client_toolkit::shell::xdg::popup::{Popup, PopupConfigure, PopupHandler};
use smithay_client_toolkit::shell::xdg::window::{Window, WindowConfigure, WindowHandler};
use smithay_client_toolkit::shell::xdg::{XdgPositioner, XdgShell, XdgSurface};
use smithay_client_toolkit::shm::Shm;
use smithay_client_toolkit::{delegate_registry, registry_handlers};
use wayland_client::globals::{BindError, GlobalError, registry_queue_init};
use wayland_client::protocol::{wl_keyboard, wl_output, wl_pointer, wl_seat, wl_surface};
use wayland_client::{ConnectError, Connection, Proxy, QueueHandle};
use wayland_protocols::wp::idle_inhibit::zv1::client::zwp_idle_inhibit_manager_v1::ZwpIdleInhibitManagerV1;
use wayland_protocols::wp::idle_inhibit::zv1::client::zwp_idle_inhibitor_v1::ZwpIdleInhibitorV1;

use crate::clipboard::{
    MAX_TEXT_BYTES, Pump, ReadJob, Selection, SystemClipboard, decode_text, is_own_offer, pick_text_mime,
};
use crate::control::{self, DataControl};
use crate::idle_inhibit::IdleInhibitGlobal;
use crate::lazy::LazyGate;
use crate::lock::{LockConfig, LockMachine, LockPhase, OnLocked, reconcile};
use crate::output::OutputInfo;
use crate::popup::PopupConfig;
use crate::scale::{FractionalData, ScaleGlobal, ScaleManagers, ScaleObjects};
use crate::surface::{LayerConfig, Role, SurfaceWindow, Toplevel};
use crate::window::{WindowCommand, WindowConfig, WindowState, resolve_size};

slotmap::new_key_type! {
    /// Identifies a surface of the application.
    pub struct SurfaceId;
}

/// Errors of the Wayland application.
#[derive(Debug, thiserror::Error)]
pub enum AppError {
    /// Connecting to the compositor failed.
    #[error("cannot connect to the Wayland compositor: {0}")]
    Connect(#[from] ConnectError),
    /// Reading the compositor's globals failed.
    #[error("cannot read Wayland globals: {0}")]
    Globals(#[from] GlobalError),
    /// A required protocol is missing or could not be bound.
    #[error("compositor lacks a required protocol: {0}")]
    Bind(#[from] BindError),
    /// The event loop failed.
    #[error("event loop error: {0}")]
    EventLoop(#[from] calloop::Error),
    /// Registering an event source failed.
    #[error("cannot register event source: {0}")]
    Insert(String),
    /// The GPU could not be initialised.
    #[error(transparent)]
    Gpu(#[from] GpuError),
    /// The compositor does not support popups.
    #[error("the compositor does not offer xdg_wm_base, so popups are unavailable")]
    NoPopups,
    /// The surface does not exist.
    #[error("the parent surface no longer exists")]
    UnknownSurface,
    /// The compositor does not support toplevel windows.
    #[error("the compositor does not offer xdg_wm_base, so toplevel windows are unavailable")]
    NoToplevels,
    /// The compositor does not support `ext-session-lock-v1`.
    #[error("the compositor does not offer ext_session_lock_manager_v1, so the session cannot be locked")]
    NoSessionLock,
    /// A session lock is already held or being requested.
    #[error("the session is already locked or being locked by this application")]
    AlreadyLocked,
    /// Creating a popup failed.
    #[error("cannot create popup: {0}")]
    Popup(String),
}

struct PopupRequest {
    parent: SurfaceId,
    config: PopupConfig,
    build: Box<dyn FnOnce(&mut Scene)>,
}

/// Cloneable handle that queues requests for the application from inside callbacks.
#[derive(Clone, Default)]
pub struct AppHandle {
    popups: std::rc::Rc<std::cell::RefCell<Vec<PopupRequest>>>,
    idle_inhibits: std::rc::Rc<std::cell::RefCell<Vec<(SurfaceId, bool)>>>,
    unlock: std::rc::Rc<std::cell::Cell<bool>>,
    window_commands: std::rc::Rc<std::cell::RefCell<Vec<(SurfaceId, WindowCommand)>>>,
    drags: std::rc::Rc<std::cell::RefCell<Vec<crate::dnd::DragRequest>>>,
}

impl AppHandle {
    /// Queues a popup of `parent`; `build` fills its scene.
    pub fn open_popup(&self, parent: SurfaceId, config: PopupConfig, build: impl FnOnce(&mut Scene) + 'static) {
        self.popups.borrow_mut().push(PopupRequest { parent, config, build: Box::new(build) });
    }

    /// Queues turning the idle inhibitor of `surface` on or off.
    pub fn set_idle_inhibit(&self, surface: SurfaceId, inhibit: bool) {
        self.idle_inhibits.borrow_mut().push((surface, inhibit));
    }

    /// Queues releasing the session lock taken with [`App::lock`]; ignored when no lock is held.
    pub fn unlock(&self) {
        self.unlock.set(true);
    }

    /// Queues a drag out of `surface` carrying `data`, started by the pointer press that has the input focus. The signal reports the outcome; a drag the compositor cannot start (no seat, no data device) is cancelled at once.
    pub fn start_drag(&self, surface: SurfaceId, data: DragData, actions: DropActions) -> Signal<DragOutcome> {
        let outcome = Signal::new(DragOutcome::Cancelled);
        self.drags.borrow_mut().push(crate::dnd::DragRequest { surface, data, actions, outcome: outcome.clone() });
        outcome
    }

    /// Queues `command` for the toplevel window `surface`; commands for other surfaces are ignored.
    pub fn window_command(&self, surface: SurfaceId, command: WindowCommand) {
        self.window_commands.borrow_mut().push((surface, command));
    }
}

type VariantBuild = Box<dyn FnMut(&mut Scene, &OutputInfo)>;

struct NewWindow {
    role: Role,
    wgpu_surface: acanthus_render::wgpu::Surface<'static>,
    scale_objects: Option<ScaleObjects>,
    parent: Option<SurfaceId>,
    toplevel: Option<Toplevel>,
    clear: acanthus_core::Color,
    size: (u32, u32),
}

struct Lock {
    session: Option<SessionLock>,
    machine: LockMachine,
    phase: Signal<LockPhase>,
    config: LockConfig,
    build: VariantBuild,
    surfaces: Vec<(u32, SurfaceId)>,
}

struct Variants {
    config: Box<dyn Fn(&OutputInfo) -> LayerConfig>,
    build: VariantBuild,
    surfaces: Vec<(u32, SurfaceId)>,
    lazy: Option<(Signal<bool>, LazyGate)>,
}

/// The state of the application: surfaces, outputs, input and the GPU, passed to every event source.
pub struct State {
    conn: Connection,
    pub(crate) qh: QueueHandle<State>,
    registry: RegistryState,
    seat_state: SeatState,
    output_state: OutputState,
    compositor: CompositorState,
    layer_shell: LayerShell,
    xdg_shell: Option<XdgShell>,
    clipboard: SystemClipboard,
    data_control: Option<Box<dyn DataControl>>,
    session_lock_state: SessionLockState,
    lock: Option<Lock>,
    scale_managers: Option<ScaleManagers>,
    idle_inhibit_manager: Option<ZwpIdleInhibitManagerV1>,
    idle_inhibitors: HashMap<SurfaceId, ZwpIdleInhibitorV1>,
    pub(crate) seats: Vec<wl_seat::WlSeat>,
    pub(crate) input_serial: Option<u32>,
    outputs: Signal<Vec<OutputInfo>>,
    pointer_output: Signal<Option<u32>>,
    variants: Vec<Variants>,
    handle: AppHandle,
    cursor_shapes: Option<CursorShapeManager>,
    pointers: Vec<(wl_pointer::WlPointer, Option<WpCursorShapeDeviceV1>)>,
    keyboards: Vec<wl_keyboard::WlKeyboard>,
    keyboard_focus: Option<SurfaceId>,
    modifiers: Modifiers,
    pointer_serial: u32,
    applied_cursor: CursorShape,
    pub(crate) loop_handle: calloop::LoopHandle<'static, State>,
    pub(crate) shm: Shm,
    pub(crate) captures: crate::capture::Captures,
    pub(crate) dnd: crate::dnd::Dnd,
    pub(crate) surfaces: slotmap::SlotMap<SurfaceId, SurfaceWindow>,
    gpu: Option<Gpu>,
    fonts: Fonts,
    images: ImageStore,
    last_tick: Option<Instant>,
    exit: bool,
}

/// A Wayland application: its event loop and state.
pub struct App {
    event_loop: EventLoop<'static, State>,
    state: State,
}

impl App {
    /// Connects to the compositor and prepares the GPU.
    pub fn new() -> Result<Self, AppError> {
        let conn = Connection::connect_to_env()?;
        let (globals, queue) = registry_queue_init(&conn)?;
        let qh = queue.handle();
        let event_loop = EventLoop::try_new()?;
        WaylandSource::new(conn.clone(), queue)
            .insert(event_loop.handle())
            .map_err(|e| AppError::Insert(e.error.to_string()))?;

        let (image_tx, image_rx) = calloop::channel::channel::<()>();
        let image_tx = std::sync::Mutex::new(image_tx);
        let images = ImageStore::with_waker(
            std::sync::Arc::new(acanthus_scene::IconLookup::from_environment()),
            std::sync::Arc::new(move || {
                let _ = image_tx.lock().map(|tx| tx.send(()));
            }),
        );
        event_loop
            .handle()
            .insert_source(image_rx, |_, _, state: &mut State| {
                state.images.poll();
            })
            .map_err(|e| AppError::Insert(e.error.to_string()))?;

        let state = State {
            registry: RegistryState::new(&globals),
            seat_state: SeatState::new(&globals, &qh),
            output_state: OutputState::new(&globals, &qh),
            compositor: CompositorState::bind(&globals, &qh)?,
            layer_shell: LayerShell::bind(&globals, &qh)?,
            xdg_shell: XdgShell::bind(&globals, &qh).ok(),
            clipboard: SystemClipboard::new(),
            data_control: control::bind(&globals, &qh),
            session_lock_state: SessionLockState::new(&globals, &qh),
            lock: None,
            scale_managers: bind_scale_managers(&globals, &qh),
            idle_inhibit_manager: globals.bind(&qh, 1..=1, IdleInhibitGlobal).ok(),
            idle_inhibitors: HashMap::new(),
            seats: Vec::new(),
            input_serial: None,
            outputs: Signal::new(Vec::new()),
            pointer_output: Signal::new(None),
            variants: Vec::new(),
            handle: AppHandle::default(),
            cursor_shapes: CursorShapeManager::bind(&globals, &qh).ok(),
            pointers: Vec::new(),
            keyboards: Vec::new(),
            keyboard_focus: None,
            modifiers: Modifiers::default(),
            pointer_serial: 0,
            applied_cursor: CursorShape::Default,
            loop_handle: event_loop.handle(),
            shm: Shm::bind(&globals, &qh)?,
            captures: crate::capture::Captures::bind(&globals, &qh),
            dnd: crate::dnd::Dnd::bind(&globals, &qh),
            surfaces: slotmap::SlotMap::with_key(),
            gpu: None,
            fonts: Fonts::new(),
            images,
            last_tick: None,
            exit: false,
            conn,
            qh,
        };
        Ok(App { event_loop, state })
    }

    /// Fonts shared by all scenes.
    pub fn fonts(&self) -> &Fonts {
        &self.state.fonts
    }

    /// Images shared by all scenes.
    pub fn images(&self) -> &ImageStore {
        &self.state.images
    }

    /// Handle for adding event sources to the loop.
    pub fn loop_handle(&self) -> calloop::LoopHandle<'static, State> {
        self.event_loop.handle()
    }

    /// Creates a layer surface; `build` fills its scene.
    pub fn layer_surface(
        &mut self,
        config: LayerConfig,
        build: impl FnOnce(&mut Scene),
    ) -> Result<SurfaceId, AppError> {
        self.state.create_layer(config, build)
    }

    /// Creates a popup of `parent`; `build` fills its scene.
    pub fn popup(
        &mut self,
        parent: SurfaceId,
        config: PopupConfig,
        build: impl FnOnce(&mut Scene),
    ) -> Result<SurfaceId, AppError> {
        self.state.create_popup(parent, config, build)
    }

    /// Creates a toplevel window; `build` fills its scene. The returned signal follows the compositor's window state.
    pub fn window(
        &mut self,
        config: WindowConfig,
        build: impl FnOnce(&mut Scene),
    ) -> Result<(SurfaceId, Signal<WindowState>), AppError> {
        self.state.create_window(config, build)
    }

    /// Locks the session: asks the compositor for `ext-session-lock-v1` and keeps one lock surface per output, built with `build`, recreated as outputs come and go. The signal follows the lock; release it with [`AppHandle::unlock`]. When the process dies the compositor keeps the session locked.
    pub fn lock(
        &mut self,
        config: LockConfig,
        build: impl FnMut(&mut Scene, &OutputInfo) + 'static,
    ) -> Result<Signal<LockPhase>, AppError> {
        self.state.start_lock(config, Box::new(build))
    }

    /// Handle for queueing requests from callbacks.
    pub fn handle(&self) -> AppHandle {
        self.state.handle.clone()
    }

    /// Destroys a surface.
    pub fn close_surface(&mut self, id: SurfaceId) {
        self.state.remove_surface(id);
    }

    /// Captures `output` into frames: one frame, or a live feed up to `config.max_fps`. Uses `ext-image-copy-capture-v1` or `wlr-screencopy-unstable-v1` with shared memory.
    pub fn capture(
        &mut self,
        output: &OutputInfo,
        config: crate::capture::CaptureConfig,
    ) -> Result<crate::capture::Capture, crate::capture::CaptureError> {
        crate::capture::start(&mut self.state, output.output.clone(), config)
    }

    /// True when the compositor offers a screen capture protocol.
    pub fn can_capture(&self) -> bool {
        self.state.captures.is_supported()
    }

    /// The desktop clipboard; every scene's text widgets already use it.
    pub fn clipboard(&self) -> SystemClipboard {
        self.state.clipboard.clone()
    }

    /// Signal of the connected outputs.
    pub fn outputs(&self) -> Signal<Vec<OutputInfo>> {
        self.state.outputs.clone()
    }

    /// Signal of the output the pointer is on, by output id.
    pub fn pointer_output(&self) -> Signal<Option<u32>> {
        self.state.pointer_output.clone()
    }

    /// Keeps one layer surface per output, built with `build`; `config` decides its placement. Surfaces follow outputs as they come and go.
    pub fn per_output(
        &mut self,
        config: impl Fn(&OutputInfo) -> LayerConfig + 'static,
        build: impl FnMut(&mut Scene, &OutputInfo) + 'static,
    ) {
        self.state.variants.push(Variants {
            config: Box::new(config),
            build: Box::new(build),
            surfaces: Vec::new(),
            lazy: None,
        });
        self.state.sync_variants();
    }

    /// Like [`App::per_output`], but the surfaces exist only while `open` is true and for `linger` after it turns false.
    pub fn per_output_lazy(
        &mut self,
        open: Signal<bool>,
        linger: Duration,
        config: impl Fn(&OutputInfo) -> LayerConfig + 'static,
        build: impl FnMut(&mut Scene, &OutputInfo) + 'static,
    ) {
        self.state.variants.push(Variants {
            config: Box::new(config),
            build: Box::new(build),
            surfaces: Vec::new(),
            lazy: Some((open, LazyGate::new(linger))),
        });
        self.state.sync_variants();
    }

    /// Scene of a surface.
    pub fn scene_mut(&mut self, id: SurfaceId) -> Option<&mut Scene> {
        self.state.surfaces.get_mut(id).map(|s| &mut s.scene)
    }

    /// Calls `f` every `interval`.
    pub fn every(&self, interval: Duration, mut f: impl FnMut() + 'static) -> Result<(), AppError> {
        self.event_loop
            .handle()
            .insert_source(Timer::from_duration(interval), move |deadline, _, _| {
                f();
                TimeoutAction::ToInstant(deadline + interval)
            })
            .map(|_| ())
            .map_err(|e| AppError::Insert(e.error.to_string()))
    }

    /// Calls `f` once after `delay`.
    pub fn after(&self, delay: Duration, f: impl FnOnce() + 'static) -> Result<(), AppError> {
        let mut f = Some(f);
        self.event_loop
            .handle()
            .insert_source(Timer::from_duration(delay), move |_, _, _| {
                if let Some(f) = f.take() {
                    f();
                }
                TimeoutAction::Drop
            })
            .map(|_| ())
            .map_err(|e| AppError::Insert(e.error.to_string()))
    }

    /// Runs the event loop until the application exits.
    pub fn run(mut self) -> Result<(), AppError> {
        loop {
            let idle_animation = acanthus_anim::is_active() && !self.state.surfaces.values().any(|s| s.frame_pending);
            let frame_timeout = idle_animation.then(|| Duration::from_millis(16));
            let timeout = match (frame_timeout, self.state.lazy_deadline()) {
                (Some(frame), Some(at)) => Some(frame.min(at.saturating_duration_since(Instant::now()))),
                (None, Some(at)) => Some(at.saturating_duration_since(Instant::now())),
                (frame, None) => frame,
            };
            self.event_loop.dispatch(timeout, &mut self.state)?;
            self.state.update();
            if self.state.exit
                || (self.state.surfaces.is_empty()
                    && self.state.variants.is_empty()
                    && self.state.lock.is_none()
                    && self.state.captures.is_empty())
            {
                return Ok(());
            }
        }
    }
}

impl State {
    fn create_wgpu_surface(
        &mut self,
        wl: &wl_surface::WlSurface,
    ) -> Result<acanthus_render::wgpu::Surface<'static>, AppError> {
        let display = NonNull::new(self.conn.backend().display_ptr().cast()).expect("null wl_display");
        let window = NonNull::new(wl.id().as_ptr().cast()).expect("null wl_surface");
        let target = acanthus_render::wgpu::SurfaceTargetUnsafe::RawHandle {
            raw_display_handle: Some(RawDisplayHandle::Wayland(WaylandDisplayHandle::new(display))),
            raw_window_handle: RawWindowHandle::Wayland(WaylandWindowHandle::new(window)),
        };
        let instance = match &self.gpu {
            Some(gpu) => gpu.instance.clone(),
            None => Gpu::instance(),
        };
        let surface = unsafe { instance.create_surface_unsafe(target) }.map_err(GpuError::from)?;
        if self.gpu.is_none() {
            self.gpu = Some(pollster::block_on(Gpu::new(instance, Some(&surface)))?);
        }
        Ok(surface)
    }

    fn update(&mut self) {
        if acanthus_anim::is_active() {
            let now = Instant::now();
            let dt = self.last_tick.map_or(0.0, |t| (now - t).as_secs_f32().min(0.1));
            self.last_tick = Some(now);
            acanthus_anim::tick(dt);
        } else {
            self.last_tick = None;
        }
        let inhibits: Vec<(SurfaceId, bool)> = self.handle.idle_inhibits.borrow_mut().drain(..).collect();
        for (surface, inhibit) in inhibits {
            self.apply_idle_inhibit(surface, inhibit);
        }
        self.apply_unlock();
        self.apply_clipboard();
        crate::capture::sweep(self);
        if let Some(seat) = self.seats.first().cloned() {
            let qh = self.qh.clone();
            self.dnd.ensure_device(&seat, &qh);
        }
        let drags: Vec<crate::dnd::DragRequest> = self.handle.drags.borrow_mut().drain(..).collect();
        for request in drags {
            crate::dnd::start_drag(self, request);
        }
        let commands: Vec<(SurfaceId, WindowCommand)> = self.handle.window_commands.borrow_mut().drain(..).collect();
        for (surface, command) in commands {
            self.apply_window_command(surface, command);
        }
        let requests: Vec<PopupRequest> = self.handle.popups.borrow_mut().drain(..).collect();
        for request in requests {
            if let Err(e) = self.create_popup(request.parent, request.config, request.build) {
                tracing::warn!("cannot open popup: {e}");
            }
        }
        if self.variants.iter().any(|v| v.lazy.is_some()) {
            self.sync_variants();
        }
        self.apply_input_regions();
        self.apply_keyboard_modes();
        self.apply_layer_levels();
        self.apply_layer_geometry();
        let Some(gpu) = self.gpu.as_ref() else { return };
        for surface in self.surfaces.values_mut() {
            if surface.wants_frame() {
                surface.draw(gpu, &self.qh);
            }
        }
    }

    fn apply_input_regions(&mut self) {
        for surface in self.surfaces.values_mut() {
            let wanted = surface.scene.surface_input_region();
            if wanted == surface.applied_region {
                continue;
            }
            let wl = surface.wl_surface().clone();
            match &wanted {
                None => wl.set_input_region(None),
                Some(rects) => match smithay_client_toolkit::compositor::Region::new(&self.compositor) {
                    Ok(region) => {
                        for r in rects.iter().filter(|r| r.w > 0.0 && r.h > 0.0) {
                            region.add(r.x.floor() as i32, r.y.floor() as i32, r.w.ceil() as i32, r.h.ceil() as i32);
                        }
                        wl.set_input_region(Some(region.wl_region()));
                    }
                    Err(e) => {
                        tracing::warn!("cannot create input region: {e}");
                        continue;
                    }
                },
            }
            wl.commit();
            surface.applied_region = wanted;
        }
    }

    fn apply_layer_geometry(&mut self) {
        for surface in self.surfaces.values_mut() {
            let Some(wanted) = surface.scene.surface_layer_geometry() else { continue };
            if Some(wanted) == surface.applied_geometry {
                continue;
            }
            if let Role::Layer(layer) = &surface.role {
                let mut anchor = Anchor::empty();
                for (set, edge) in [
                    (wanted.top, Anchor::TOP),
                    (wanted.bottom, Anchor::BOTTOM),
                    (wanted.left, Anchor::LEFT),
                    (wanted.right, Anchor::RIGHT),
                ] {
                    if set {
                        anchor |= edge;
                    }
                }
                layer.set_anchor(anchor);
                layer.set_size(wanted.size.0, wanted.size.1);
                let [top, right, bottom, left] = wanted.margin;
                layer.set_margin(top, right, bottom, left);
                layer.set_exclusive_zone(wanted.exclusive_zone);
                layer.wl_surface().commit();
            }
            surface.applied_geometry = Some(wanted);
        }
    }

    fn apply_layer_levels(&mut self) {
        for surface in self.surfaces.values_mut() {
            let wanted = surface.scene.layer_raised().get_untracked();
            if wanted == surface.applied_raised {
                continue;
            }
            if let (Some(raised), Role::Layer(layer)) = (wanted, &surface.role) {
                layer.set_layer(if raised { Layer::Top } else { Layer::Bottom });
                layer.wl_surface().commit();
            }
            surface.applied_raised = wanted;
        }
    }

    fn apply_keyboard_modes(&mut self) {
        for surface in self.surfaces.values_mut() {
            let wanted = surface.scene.keyboard_on_demand().get_untracked();
            if wanted == surface.applied_keyboard {
                continue;
            }
            let Some(on_demand) = wanted else {
                surface.applied_keyboard = wanted;
                continue;
            };
            if let Role::Layer(layer) = &surface.role {
                layer.set_keyboard_interactivity(if on_demand {
                    KeyboardInteractivity::OnDemand
                } else {
                    KeyboardInteractivity::None
                });
                layer.wl_surface().commit();
            }
            surface.applied_keyboard = wanted;
        }
    }

    pub(crate) fn surface_by_wl(&mut self, wl: &wl_surface::WlSurface) -> Option<SurfaceId> {
        self.surfaces.iter().find(|(_, s)| s.wl_surface() == wl).map(|(id, _)| id)
    }

    fn attach_scale(&self, wl: &wl_surface::WlSurface) -> Option<ScaleObjects> {
        let managers = self.scale_managers.as_ref()?;
        let viewport = managers.viewporter.get_viewport(wl, &self.qh, ScaleGlobal);
        let fractional = managers.fractional.get_fractional_scale(wl, &self.qh, FractionalData { surface: wl.clone() });
        Some(ScaleObjects { fractional, viewport })
    }

    fn insert_window(&mut self, window: NewWindow, build: impl FnOnce(&mut Scene)) -> SurfaceId {
        let NewWindow { role, wgpu_surface, scale_objects, parent, toplevel, clear, size } = window;
        let mut scene = Scene::with_images(self.fonts.clone(), self.images.clone());
        scene.set_window_focus(false);
        scene.set_clipboard_backend(std::rc::Rc::new(self.clipboard.clone()));
        build(&mut scene);
        self.surfaces.insert(SurfaceWindow {
            wgpu_surface,
            renderer: None,
            config: None,
            scale_objects,
            role,
            parent,
            toplevel,
            entered: Vec::new(),
            scene,
            clear,
            requested: size,
            size,
            scale: 1.0,
            fractional: false,
            configured: false,
            frame_pending: false,
            dirty: true,
            applied_region: None,
            applied_keyboard: None,
            applied_raised: None,
            applied_geometry: None,
        })
    }

    fn create_layer(&mut self, config: LayerConfig, build: impl FnOnce(&mut Scene)) -> Result<SurfaceId, AppError> {
        let wl = self.compositor.create_surface(&self.qh);
        let scale_objects = self.attach_scale(&wl);
        let layer = self.layer_shell.create_layer_surface(
            &self.qh,
            wl,
            config.layer,
            Some(config.namespace.clone()),
            config.output.as_ref(),
        );
        layer.set_anchor(config.anchor);
        layer.set_size(config.width, config.height);
        layer.set_exclusive_zone(config.exclusive_zone);
        let [top, right, bottom, left] = config.margin;
        layer.set_margin(top, right, bottom, left);
        layer.set_keyboard_interactivity(config.keyboard);
        layer.commit();

        let wgpu_surface = self.create_wgpu_surface(layer.wl_surface())?;
        let size = (config.width, config.height);
        let window = NewWindow {
            role: Role::Layer(layer),
            wgpu_surface,
            scale_objects,
            parent: None,
            toplevel: None,
            clear: config.clear,
            size,
        };
        Ok(self.insert_window(window, build))
    }

    fn create_window(
        &mut self,
        config: WindowConfig,
        build: impl FnOnce(&mut Scene),
    ) -> Result<(SurfaceId, Signal<WindowState>), AppError> {
        let shell = self.xdg_shell.as_ref().ok_or(AppError::NoToplevels)?;
        let wl = self.compositor.create_surface(&self.qh);
        let scale_objects = self.attach_scale(&wl);
        let window = shell.create_window(wl, config.decorations.wire(), &self.qh);
        window.set_title(config.title.clone());
        window.set_app_id(config.app_id.clone());
        window.set_min_size(config.min_size);
        window.set_max_size(config.max_size);
        window.commit();

        let wgpu_surface = self.create_wgpu_surface(window.wl_surface())?;
        let state = Signal::new(WindowState::default());
        let toplevel = Toplevel {
            state: state.clone(),
            on_close: config.on_close.clone(),
            min_size: config.min_size,
            max_size: config.max_size,
        };
        let new = NewWindow {
            role: Role::Window(window),
            wgpu_surface,
            scale_objects,
            parent: None,
            toplevel: Some(toplevel),
            clear: config.clear,
            size: config.size,
        };
        Ok((self.insert_window(new, build), state))
    }

    fn start_lock(&mut self, config: LockConfig, build: VariantBuild) -> Result<Signal<LockPhase>, AppError> {
        let mut machine = LockMachine::default();
        if self.lock.is_some() || !machine.request() {
            return Err(AppError::AlreadyLocked);
        }
        let session = self.session_lock_state.lock(&self.qh).map_err(|_| AppError::NoSessionLock)?;
        let phase = Signal::new(machine.phase());
        self.lock =
            Some(Lock { session: Some(session), machine, phase: phase.clone(), config, build, surfaces: Vec::new() });
        self.sync_lock_surfaces();
        Ok(phase)
    }

    fn sync_lock_surfaces(&mut self) {
        let Some(mut lock) = self.lock.take() else { return };
        let infos = self.outputs.get_untracked();
        let live = lock.session.is_some();
        let wanted: Vec<u32> = if live { infos.iter().map(|o| o.id).collect() } else { Vec::new() };
        let existing: Vec<u32> = lock.surfaces.iter().map(|(o, _)| *o).collect();
        let (add, remove) = reconcile(&existing, &wanted);
        for output in remove {
            if let Some(pos) = lock.surfaces.iter().position(|(o, _)| *o == output) {
                let (_, id) = lock.surfaces.remove(pos);
                self.remove_surface(id);
            }
        }
        for output in add {
            let Some(info) = infos.iter().find(|o| o.id == output) else { continue };
            let Some(session) = lock.session.as_ref() else { continue };
            let wl = self.compositor.create_surface(&self.qh);
            let scale_objects = self.attach_scale(&wl);
            let surface = session.create_lock_surface(wl, &info.output, &self.qh);
            let created = self.create_wgpu_surface(surface.wl_surface()).map(|wgpu_surface| {
                let window = NewWindow {
                    role: Role::Lock(surface),
                    wgpu_surface,
                    scale_objects,
                    parent: None,
                    toplevel: None,
                    clear: lock.config.clear,
                    size: (info.size.0.max(0) as u32, info.size.1.max(0) as u32),
                };
                self.insert_window(window, |scene| (lock.build)(scene, info))
            });
            match created {
                Ok(id) => lock.surfaces.push((output, id)),
                Err(e) => tracing::warn!(output, "cannot create lock surface: {e}"),
            }
        }
        self.lock = Some(lock);
    }

    fn apply_clipboard(&mut self) {
        let Some(control) = self.data_control.as_mut() else { return };
        if !control.has_device() {
            let Some(seat) = self.seats.first() else { return };
            control.ensure_device(seat, &self.qh);
        }
        let Some(text) = self.clipboard.shared.pending.borrow_mut().take() else { return };
        self.clipboard.shared.expect_own.set(true);
        control.offer(&text, &self.qh);
    }

    pub(crate) fn on_data_selection(
        &mut self,
        mimes: Option<Vec<String>>,
        receive: &dyn Fn(&str) -> std::io::Result<std::fs::File>,
    ) {
        let shared = &self.clipboard.shared;
        let generation = shared.generation.get() + 1;
        shared.generation.set(generation);
        let Some(mimes) = mimes else {
            shared.expect_own.set(false);
            shared.text.borrow_mut().clear();
            shared.selection.set(None);
            return;
        };
        if shared.expect_own.replace(false) && is_own_offer(&mimes) {
            return;
        }
        let mime = pick_text_mime(&mimes);
        if mime.is_none() {
            shared.text.borrow_mut().clear();
        }
        shared.selection.set(Some(Selection { mime_types: mimes, text: None }));
        let Some(mime) = mime else { return };
        let file = match receive(mime) {
            Ok(file) => file,
            Err(e) => {
                tracing::warn!("cannot read the clipboard: {e}");
                return;
            }
        };
        let shared = self.clipboard.shared.clone();
        let mut job = ReadJob::default();
        let source = calloop::generic::Generic::new(file, calloop::Interest::READ, calloop::Mode::Level);
        let inserted = self.loop_handle.insert_source(source, move |_, file, _: &mut State| {
            let reader = &mut &**file;
            match job.pump(reader, MAX_TEXT_BYTES) {
                Ok(Pump::Pending) => Ok(calloop::PostAction::Continue),
                Ok(Pump::Done(bytes)) => {
                    if shared.generation.get() == generation {
                        let text = decode_text(bytes);
                        *shared.text.borrow_mut() = text.clone();
                        if let Some(mut selection) = shared.selection.get_untracked() {
                            selection.text = Some(text);
                            shared.selection.set(Some(selection));
                        }
                    }
                    Ok(calloop::PostAction::Remove)
                }
                Ok(Pump::TooLarge) => {
                    tracing::warn!("clipboard text exceeds the size limit; ignored");
                    Ok(calloop::PostAction::Remove)
                }
                Err(e) => {
                    tracing::warn!("clipboard read failed: {e}");
                    Ok(calloop::PostAction::Remove)
                }
            }
        });
        if let Err(e) = inserted {
            tracing::warn!("cannot watch the clipboard pipe: {}", e.error);
        }
    }

    fn apply_unlock(&mut self) {
        if !self.handle.unlock.replace(false) {
            return;
        }
        let Some(lock) = self.lock.as_mut() else { return };
        if !lock.machine.unlock() {
            return;
        }
        if let Some(session) = lock.session.take() {
            session.unlock();
        }
        lock.phase.set(LockPhase::Unlocked);
        self.release_lock();
    }

    fn release_lock(&mut self) {
        let Some(lock) = self.lock.take() else { return };
        for (_, id) in lock.surfaces {
            self.remove_surface(id);
        }
    }

    fn apply_window_command(&mut self, id: SurfaceId, command: WindowCommand) {
        let Some(surface) = self.surfaces.get(id) else { return };
        let Role::Window(window) = &surface.role else { return };
        let seat_serial = self.seats.first().zip(self.input_serial);
        match command {
            WindowCommand::SetTitle(title) => window.set_title(title),
            WindowCommand::Move => match seat_serial {
                Some((seat, serial)) => window.move_(seat, serial),
                None => tracing::warn!("window move skipped: no seat or input serial yet"),
            },
            WindowCommand::Resize(edge) => match seat_serial {
                Some((seat, serial)) => window.resize(seat, serial, edge.wire()),
                None => tracing::warn!("window resize skipped: no seat or input serial yet"),
            },
            WindowCommand::Maximize => window.set_maximized(),
            WindowCommand::Unmaximize => window.unset_maximized(),
            WindowCommand::Fullscreen => window.set_fullscreen(None),
            WindowCommand::Unfullscreen => window.unset_fullscreen(),
            WindowCommand::Minimize => window.set_minimized(),
            WindowCommand::Close => self.request_window_close(id),
        }
    }

    fn request_window_close(&mut self, id: SurfaceId) {
        let handler = self.surfaces.get(id).and_then(|s| s.toplevel.as_ref()).map(|t| t.on_close.clone());
        match handler {
            Some(Some(handler)) => (handler.borrow_mut())(),
            Some(None) => self.remove_surface(id),
            None => {}
        }
    }

    fn create_popup(
        &mut self,
        parent: SurfaceId,
        config: PopupConfig,
        build: impl FnOnce(&mut Scene),
    ) -> Result<SurfaceId, AppError> {
        let shell = self.xdg_shell.as_ref().ok_or(AppError::NoPopups)?;
        let parent_window = self.surfaces.get(parent).ok_or(AppError::UnknownSurface)?;
        let positioner = XdgPositioner::new(shell).map_err(|e| AppError::Popup(e.to_string()))?;
        let (ax, ay, aw, ah) = config.anchor_rect;
        positioner.set_size(config.size.0.max(1) as i32, config.size.1.max(1) as i32);
        positioner.set_anchor_rect(ax, ay, aw.max(1), ah.max(1));
        positioner.set_anchor(config.anchor.anchor());
        positioner.set_gravity(config.gravity.gravity());
        positioner.set_offset(config.offset.0, config.offset.1);
        positioner.set_constraint_adjustment(config.adjustment);

        let wl = self.compositor.create_surface(&self.qh);
        let scale_objects = self.attach_scale(&wl);
        let parent_xdg = match &parent_window.role {
            Role::Popup(popup) => Some(popup.xdg_surface().clone()),
            Role::Window(window) => Some(window.xdg_surface().clone()),
            Role::Layer(_) => None,
            Role::Lock(_) => return Err(AppError::Popup("lock surfaces cannot have popups".into())),
        };
        let popup = Popup::from_surface(parent_xdg.as_ref(), &positioner, &self.qh, wl, shell)
            .map_err(|e| AppError::Popup(e.to_string()))?;
        if let Role::Layer(layer) = &parent_window.role {
            layer.get_popup(popup.xdg_popup());
        }
        if config.grab {
            match (self.seats.first(), self.input_serial) {
                (Some(seat), Some(serial)) => popup.xdg_popup().grab(seat, serial),
                _ => tracing::warn!("popup grab skipped: no seat or input serial yet"),
            }
        }
        popup.wl_surface().commit();

        let wl = popup.wl_surface().clone();
        let wgpu_surface = self.create_wgpu_surface(&wl)?;
        let window = NewWindow {
            role: Role::Popup(popup),
            wgpu_surface,
            scale_objects,
            parent: Some(parent),
            toplevel: None,
            clear: config.clear,
            size: config.size,
        };
        Ok(self.insert_window(window, build))
    }

    fn apply_idle_inhibit(&mut self, id: SurfaceId, inhibit: bool) {
        if !inhibit {
            if let Some(inhibitor) = self.idle_inhibitors.remove(&id) {
                inhibitor.destroy();
            }
            return;
        }
        if self.idle_inhibitors.contains_key(&id) {
            return;
        }
        let Some(manager) = self.idle_inhibit_manager.as_ref() else {
            tracing::warn!("the compositor lacks zwp_idle_inhibit_manager_v1");
            return;
        };
        let Some(surface) = self.surfaces.get(id) else { return };
        let inhibitor = manager.create_inhibitor(surface.wl_surface(), &self.qh, IdleInhibitGlobal);
        self.idle_inhibitors.insert(id, inhibitor);
    }

    fn remove_surface(&mut self, id: SurfaceId) {
        if let Some(inhibitor) = self.idle_inhibitors.remove(&id) {
            inhibitor.destroy();
        }
        let children: Vec<SurfaceId> =
            self.surfaces.iter().filter(|(_, s)| s.parent == Some(id)).map(|(child, _)| child).collect();
        for child in children {
            self.remove_surface(child);
        }
        if self.keyboard_focus == Some(id) {
            self.keyboard_focus = None;
        }
        self.surfaces.remove(id);
    }

    fn refresh_outputs(&mut self, destroyed: Option<&wl_output::WlOutput>) {
        let infos: Vec<OutputInfo> = self
            .output_state
            .outputs()
            .filter(|o| Some(o) != destroyed)
            .filter_map(|o| self.output_state.info(&o).map(|raw| OutputInfo::from_raw(&raw, o)))
            .collect();
        self.outputs.set(infos);
        self.sync_variants();
        self.sync_lock_surfaces();
    }

    fn lazy_deadline(&self) -> Option<Instant> {
        self.variants.iter().filter_map(|v| v.lazy.as_ref()?.1.deadline()).min()
    }

    fn sync_variants(&mut self) {
        let infos = self.outputs.get_untracked();
        let mut variants = std::mem::take(&mut self.variants);
        let now = Instant::now();
        for variant in &mut variants {
            let mapped = match variant.lazy.as_mut() {
                Some((open, gate)) => gate.step(open.get_untracked(), now),
                None => true,
            };
            variant.surfaces.retain(|(output, id)| {
                let alive = mapped && infos.iter().any(|o| o.id == *output) && self.surfaces.contains_key(*id);
                if !alive {
                    self.remove_surface(*id);
                }
                alive
            });
            for info in infos.iter().filter(|_| mapped) {
                if variant.surfaces.iter().any(|(output, _)| *output == info.id) {
                    continue;
                }
                let config = (variant.config)(info).output(info.output.clone());
                match self.create_layer(config, |scene| (variant.build)(scene, info)) {
                    Ok(id) => variant.surfaces.push((info.id, id)),
                    Err(e) => tracing::warn!(output = info.id, "cannot create surface for output: {e}"),
                }
            }
        }
        self.variants = variants;
    }

    fn publish_surface_output(&self, id: SurfaceId) {
        let value = self.surfaces[id].entered.first().and_then(|o| {
            self.output_state.info(o).map(|raw| {
                let info = OutputInfo::from_raw(&raw, o.clone());
                acanthus_scene::SurfaceOutput {
                    name: info.name,
                    size: Vec2::new(info.size.0 as f32, info.size.1 as f32),
                }
            })
        });
        let signal = self.surfaces[id].scene.surface_output();
        if signal.get_untracked() != value {
            signal.set(value);
        }
    }

    fn output_id(&self, output: &wl_output::WlOutput) -> Option<u32> {
        self.output_state.info(output).map(|info| info.id)
    }

    pub(crate) fn set_fractional_scale(&mut self, wl: &wl_surface::WlSurface, scale: f32) {
        let Some(id) = self.surface_by_wl(wl) else { return };
        let surface = &mut self.surfaces[id];
        if surface.set_fractional_scale(scale)
            && surface.configured
            && let Some(gpu) = self.gpu.as_ref()
        {
            surface.reconfigure(gpu);
        }
    }

    fn deliver_key(&mut self, raw: &RawKeyEvent, repeat: bool) {
        let Some(id) = self.keyboard_focus else { return };
        let Some(surface) = self.surfaces.get_mut(id) else { return };
        let Some(event) = translate_key(raw, self.modifiers, repeat) else { return };
        surface.scene.key_event(&event);
    }

    fn apply_cursor(&mut self, id: SurfaceId) {
        let shape = self.surfaces[id].scene.cursor_shape();
        if shape == self.applied_cursor {
            return;
        }
        self.applied_cursor = shape;
        for (_, device) in &self.pointers {
            if let Some(device) = device {
                device.set_shape(self.pointer_serial, wire_shape(shape));
            }
        }
    }

    fn surface_by_layer(&mut self, layer: &LayerSurface) -> Option<SurfaceId> {
        self.surface_by_wl(layer.wl_surface())
    }
}

impl CompositorHandler for State {
    fn scale_factor_changed(&mut self, _: &Connection, _: &QueueHandle<Self>, wl: &wl_surface::WlSurface, factor: i32) {
        let Some(id) = self.surface_by_wl(wl) else { return };
        let surface = &mut self.surfaces[id];
        if surface.set_integer_scale(factor)
            && let (true, Some(gpu)) = (surface.configured, self.gpu.as_ref())
        {
            surface.reconfigure(gpu);
        }
    }

    fn transform_changed(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_surface::WlSurface,
        _: wl_output::Transform,
    ) {
    }

    fn frame(&mut self, _: &Connection, _: &QueueHandle<Self>, wl: &wl_surface::WlSurface, _: u32) {
        if let Some(id) = self.surface_by_wl(wl) {
            self.surfaces[id].frame_pending = false;
        }
    }

    fn surface_enter(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        wl: &wl_surface::WlSurface,
        output: &wl_output::WlOutput,
    ) {
        if let Some(id) = self.surface_by_wl(wl)
            && !self.surfaces[id].entered.contains(output)
        {
            self.surfaces[id].entered.push(output.clone());
        }
        if let Some(id) = self.surface_by_wl(wl) {
            self.publish_surface_output(id);
        }
    }

    fn surface_leave(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        wl: &wl_surface::WlSurface,
        output: &wl_output::WlOutput,
    ) {
        if let Some(id) = self.surface_by_wl(wl) {
            self.surfaces[id].entered.retain(|o| o != output);
            self.publish_surface_output(id);
        }
    }
}

impl OutputHandler for State {
    fn output_state(&mut self) -> &mut OutputState {
        &mut self.output_state
    }

    fn new_output(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {
        self.refresh_outputs(None);
    }

    fn update_output(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {
        self.refresh_outputs(None);
    }

    fn output_destroyed(&mut self, _: &Connection, _: &QueueHandle<Self>, output: wl_output::WlOutput) {
        self.refresh_outputs(Some(&output));
    }
}

impl LayerShellHandler for State {
    fn closed(&mut self, _: &Connection, _: &QueueHandle<Self>, layer: &LayerSurface) {
        if let Some(id) = self.surface_by_layer(layer) {
            self.remove_surface(id);
        }
    }

    fn configure(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        layer: &LayerSurface,
        configure: LayerSurfaceConfigure,
        _: u32,
    ) {
        let Some(id) = self.surface_by_layer(layer) else { return };
        let surface = &mut self.surfaces[id];
        let (w, h) = configure.new_size;
        surface.size = (if w == 0 { surface.requested.0 } else { w }, if h == 0 { surface.requested.1 } else { h });
        surface.configured = true;
        if let Some(gpu) = self.gpu.as_ref() {
            surface.reconfigure(gpu);
        }
    }
}

impl SessionLockHandler for State {
    fn locked(&mut self, _: &Connection, _: &QueueHandle<Self>, _: SessionLock) {
        let Some(lock) = self.lock.as_mut() else { return };
        let action = lock.machine.locked();
        lock.phase.set(lock.machine.phase());
        if action == OnLocked::UnlockNow {
            self.handle.unlock.set(true);
        }
    }

    fn finished(&mut self, _: &Connection, _: &QueueHandle<Self>, _: SessionLock) {
        let Some(lock) = self.lock.as_mut() else { return };
        lock.machine.finished();
        lock.session = None;
        lock.phase.set(lock.machine.phase());
        for (_, id) in std::mem::take(&mut lock.surfaces) {
            self.remove_surface(id);
        }
    }

    fn configure(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        lock_surface: SessionLockSurface,
        configure: SessionLockSurfaceConfigure,
        _: u32,
    ) {
        let Some(id) = self.surface_by_wl(lock_surface.wl_surface()) else { return };
        let surface = &mut self.surfaces[id];
        let (w, h) = configure.new_size;
        if w > 0 && h > 0 {
            surface.size = (w, h);
        }
        surface.configured = true;
        if let Some(gpu) = self.gpu.as_ref() {
            surface.reconfigure(gpu);
        }
    }
}

impl SeatHandler for State {
    fn seat_state(&mut self) -> &mut SeatState {
        &mut self.seat_state
    }

    fn new_seat(&mut self, _: &Connection, _: &QueueHandle<Self>, seat: wl_seat::WlSeat) {
        self.seats.push(seat);
    }

    fn new_capability(
        &mut self,
        _: &Connection,
        qh: &QueueHandle<Self>,
        seat: wl_seat::WlSeat,
        capability: Capability,
    ) {
        tracing::debug!(?capability, "seat capability");
        if capability == Capability::Keyboard {
            let callback = Box::new(|state: &mut State, _: &wl_keyboard::WlKeyboard, event: RawKeyEvent| {
                state.deliver_key(&event, true)
            });
            match self.seat_state.get_keyboard_with_repeat(qh, &seat, None, self.loop_handle.clone(), callback) {
                Ok(keyboard) => self.keyboards.push(keyboard),
                Err(e) => tracing::warn!("cannot get keyboard: {e}"),
            }
        }
        if capability == Capability::Pointer {
            match self.seat_state.get_pointer(qh, &seat) {
                Ok(pointer) => {
                    let device = self.cursor_shapes.as_ref().map(|m| m.get_shape_device(&pointer, qh));
                    self.pointers.push((pointer, device));
                }
                Err(e) => tracing::warn!("cannot get pointer: {e}"),
            }
        }
    }

    fn remove_capability(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        seat: wl_seat::WlSeat,
        capability: Capability,
    ) {
        if capability == Capability::Keyboard {
            self.keyboards.retain(|k| {
                let keep = k
                    .data::<smithay_client_toolkit::seat::keyboard::KeyboardData<State, ()>>()
                    .is_some_and(|d| d.seat() != &seat);
                if !keep {
                    k.release();
                }
                keep
            });
            if let Some(id) = self.keyboard_focus.take()
                && let Some(surface) = self.surfaces.get_mut(id)
            {
                surface.scene.set_window_focus(false);
            }
        }
        if capability == Capability::Pointer {
            self.pointers.retain(|(p, device)| {
                let keep = p
                    .data::<smithay_client_toolkit::seat::pointer::PointerData<()>>()
                    .is_some_and(|d| d.seat() != &seat);
                if !keep {
                    if let Some(d) = device {
                        d.destroy();
                    }
                    p.release();
                }
                keep
            });
        }
    }

    fn remove_seat(&mut self, _: &Connection, _: &QueueHandle<Self>, seat: wl_seat::WlSeat) {
        self.seats.retain(|s| s != &seat);
    }
}

impl PointerHandler for State {
    fn pointer_frame(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        pointer: &wl_pointer::WlPointer,
        events: &[PointerEvent],
    ) {
        for event in events {
            let Some(id) = self.surface_by_wl(&event.surface) else { continue };
            let pos = Vec2::new(event.position.0 as f32, event.position.1 as f32);
            tracing::debug!(kind = ?event.kind, x = pos.x, y = pos.y, "pointer");
            let entered_output = self.surfaces[id].entered.first().and_then(|o| self.output_id(o));
            let scene = &mut self.surfaces[id].scene;
            match event.kind {
                PointerEventKind::Enter { serial } => {
                    self.pointer_serial = serial;
                    self.input_serial = Some(serial);
                    if entered_output.is_some() {
                        self.pointer_output.set(entered_output);
                    }
                    self.applied_cursor = CursorShape::Default;
                    if let Some((_, Some(device))) = self.pointers.iter().find(|(p, _)| p == pointer) {
                        device.set_shape(serial, Shape::Default);
                    }
                    scene.pointer_motion(pos);
                }
                PointerEventKind::Motion { .. } => scene.pointer_motion(pos),
                PointerEventKind::Leave { .. } => scene.pointer_leave(),
                PointerEventKind::Press { button, serial, .. } => {
                    self.input_serial = Some(serial);
                    scene.pointer_button(MouseButton::from_linux(button), true)
                }
                PointerEventKind::Release { button, .. } => {
                    scene.pointer_button(MouseButton::from_linux(button), false)
                }
                PointerEventKind::Axis { horizontal, vertical, .. } => {
                    scene.scroll(ScrollDelta { x: horizontal.absolute as f32, y: vertical.absolute as f32 })
                }
            }
            self.apply_cursor(id);
        }
    }
}

impl KeyboardHandler for State {
    fn enter(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_keyboard::WlKeyboard,
        surface: &wl_surface::WlSurface,
        _: u32,
        _: &[u32],
        _: &[Keysym],
    ) {
        let Some(id) = self.surface_by_wl(surface) else { return };
        self.keyboard_focus = Some(id);
        self.surfaces[id].scene.set_window_focus(true);
    }

    fn leave(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_keyboard::WlKeyboard,
        surface: &wl_surface::WlSurface,
        _: u32,
    ) {
        let Some(id) = self.surface_by_wl(surface) else { return };
        if self.keyboard_focus == Some(id) {
            self.keyboard_focus = None;
        }
        self.surfaces[id].scene.set_window_focus(false);
    }

    fn press_key(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_keyboard::WlKeyboard,
        serial: u32,
        event: RawKeyEvent,
    ) {
        self.input_serial = Some(serial);
        self.deliver_key(&event, false);
    }

    fn repeat_key(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_keyboard::WlKeyboard,
        _: u32,
        event: RawKeyEvent,
    ) {
        self.deliver_key(&event, true);
    }

    fn release_key(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_keyboard::WlKeyboard,
        _: u32,
        _: RawKeyEvent,
    ) {
    }

    fn update_modifiers(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_keyboard::WlKeyboard,
        _: u32,
        modifiers: RawModifiers,
        _: smithay_client_toolkit::seat::keyboard::RawModifiers,
        _: u32,
    ) {
        self.modifiers =
            Modifiers { shift: modifiers.shift, ctrl: modifiers.ctrl, alt: modifiers.alt, logo: modifiers.logo };
    }
}

fn translate_key(raw: &RawKeyEvent, modifiers: Modifiers, repeat: bool) -> Option<KeyEvent> {
    let mut modifiers = modifiers;
    let key = match raw.keysym {
        Keysym::BackSpace => Key::Backspace,
        Keysym::Return | Keysym::KP_Enter => Key::Enter,
        Keysym::Escape => Key::Escape,
        Keysym::Tab => Key::Tab,
        Keysym::ISO_Left_Tab => {
            modifiers.shift = true;
            Key::Tab
        }
        Keysym::Left | Keysym::KP_Left => Key::Left,
        Keysym::Right | Keysym::KP_Right => Key::Right,
        Keysym::Up | Keysym::KP_Up => Key::Up,
        Keysym::Down | Keysym::KP_Down => Key::Down,
        Keysym::Home | Keysym::KP_Home => Key::Home,
        Keysym::End | Keysym::KP_End => Key::End,
        Keysym::Page_Up | Keysym::KP_Page_Up => Key::PageUp,
        Keysym::Page_Down | Keysym::KP_Page_Down => Key::PageDown,
        Keysym::Delete | Keysym::KP_Delete => Key::Delete,
        Keysym::Insert | Keysym::KP_Insert => Key::Insert,
        sym if (Keysym::F1.raw()..=Keysym::F12.raw()).contains(&sym.raw()) => {
            Key::Function((sym.raw() - Keysym::F1.raw() + 1) as u8)
        }
        sym => match sym.key_char() {
            Some(c) if !c.is_control() => Key::Char(c),
            _ if is_modifier_only(sym) => return None,
            _ => Key::Other(sym.raw()),
        },
    };
    let text = raw.utf8.as_deref().filter(|t| t.chars().all(|c| !c.is_control())).unwrap_or_default();
    let mut event = KeyEvent::new(key).with_modifiers(modifiers).with_text(text);
    event.repeat = repeat;
    Some(event)
}

fn is_modifier_only(sym: Keysym) -> bool {
    matches!(
        sym,
        Keysym::Shift_L
            | Keysym::Shift_R
            | Keysym::Control_L
            | Keysym::Control_R
            | Keysym::Alt_L
            | Keysym::Alt_R
            | Keysym::Super_L
            | Keysym::Super_R
            | Keysym::Meta_L
            | Keysym::Meta_R
            | Keysym::Caps_Lock
            | Keysym::Num_Lock
            | Keysym::ISO_Level3_Shift
    )
}

fn wire_shape(shape: CursorShape) -> Shape {
    match shape {
        CursorShape::Default => Shape::Default,
        CursorShape::Pointer => Shape::Pointer,
        CursorShape::Text => Shape::Text,
        CursorShape::Crosshair => Shape::Crosshair,
        CursorShape::Wait => Shape::Wait,
        CursorShape::Progress => Shape::Progress,
        CursorShape::Help => Shape::Help,
        CursorShape::Grab => Shape::Grab,
        CursorShape::Grabbing => Shape::Grabbing,
        CursorShape::Move => Shape::Move,
        CursorShape::NotAllowed => Shape::NotAllowed,
        CursorShape::ResizeHorizontal => Shape::EwResize,
        CursorShape::ResizeVertical => Shape::NsResize,
        CursorShape::ResizeAll => Shape::AllScroll,
    }
}

impl WindowHandler for State {
    fn request_close(&mut self, _: &Connection, _: &QueueHandle<Self>, window: &Window) {
        if let Some(id) = self.surface_by_wl(window.wl_surface()) {
            self.request_window_close(id);
        }
    }

    fn configure(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        window: &Window,
        configure: WindowConfigure,
        _: u32,
    ) {
        let Some(id) = self.surface_by_wl(window.wl_surface()) else { return };
        let surface = &mut self.surfaces[id];
        let Some(toplevel) = surface.toplevel.as_ref() else { return };
        let suggested = (configure.new_size.0.map(|w| w.get()), configure.new_size.1.map(|h| h.get()));
        surface.size = resolve_size(suggested, surface.size, toplevel.min_size, toplevel.max_size);
        let state = WindowState::from_configure(&configure);
        if toplevel.state.get_untracked() != state {
            toplevel.state.set(state);
        }
        surface.configured = true;
        if let Some(gpu) = self.gpu.as_ref() {
            surface.reconfigure(gpu);
        }
    }
}

impl PopupHandler for State {
    fn configure(&mut self, _: &Connection, _: &QueueHandle<Self>, popup: &Popup, configure: PopupConfigure) {
        let Some(id) = self.surface_by_wl(popup.wl_surface()) else { return };
        let surface = &mut self.surfaces[id];
        if configure.width > 0 && configure.height > 0 {
            surface.size = (configure.width as u32, configure.height as u32);
        }
        popup.xdg_surface().set_window_geometry(0, 0, surface.size.0 as i32, surface.size.1 as i32);
        surface.configured = true;
        if let Some(gpu) = self.gpu.as_ref() {
            surface.reconfigure(gpu);
        }
    }

    fn done(&mut self, _: &Connection, _: &QueueHandle<Self>, popup: &Popup) {
        if let Some(id) = self.surface_by_wl(popup.wl_surface()) {
            self.remove_surface(id);
        }
    }
}

fn bind_scale_managers(
    globals: &wayland_client::globals::GlobalList,
    qh: &QueueHandle<State>,
) -> Option<ScaleManagers> {
    let fractional = globals.bind(qh, 1..=1, ScaleGlobal).ok()?;
    let viewporter = globals.bind(qh, 1..=1, ScaleGlobal).ok()?;
    Some(ScaleManagers { fractional, viewporter })
}

impl smithay_client_toolkit::shm::ShmHandler for State {
    fn shm_state(&mut self) -> &mut Shm {
        &mut self.shm
    }
}

delegate_registry!(State);

impl ProvidesRegistryState for State {
    fn registry(&mut self) -> &mut RegistryState {
        &mut self.registry
    }
    registry_handlers![OutputState, SeatState];
}

smithay_client_toolkit::delegate_dispatch2!(State);
