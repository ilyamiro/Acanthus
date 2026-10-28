mod ext;
mod pace;
mod pixels;
mod wlr;

use std::cell::Cell;
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::Arc;
use std::time::{Duration, Instant};

use acanthus_core::{Signal, batch};
use acanthus_image::Decoded;
use calloop::timer::{TimeoutAction, Timer};
use smithay_client_toolkit::dispatch2::Dispatch2;
use smithay_client_toolkit::shm::raw::RawPool;
use wayland_client::globals::GlobalList;
use wayland_client::protocol::wl_buffer::WlBuffer;
use wayland_client::protocol::wl_output::WlOutput;
use wayland_client::{Connection, QueueHandle};

use crate::app::State;
use pace::Pacer;
use pixels::{PixelFormat, to_rgba};

/// How an output is captured.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CaptureConfig {
    /// Keep capturing after the first frame, up to `max_fps` frames per second.
    pub live: bool,
    /// Paint the pointer into the frames.
    pub cursor: bool,
    /// Frame rate ceiling of a live capture.
    pub max_fps: u32,
}

impl Default for CaptureConfig {
    fn default() -> Self {
        CaptureConfig { live: false, cursor: false, max_fps: 30 }
    }
}

/// Where a capture stands.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CaptureState {
    /// Waiting for the first frame.
    Starting,
    /// A live capture delivering frames.
    Streaming,
    /// A one-shot capture delivered its frame.
    Done,
    /// The capture ended without a usable frame, with the reason.
    Failed(String),
}

/// Why a capture could not start.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum CaptureError {
    /// The compositor offers neither `ext-image-copy-capture-v1` nor `wlr-screencopy-unstable-v1`.
    #[error("the compositor offers no screen capture protocol")]
    Unsupported,
}

struct Shared {
    frame: Signal<Option<Arc<Decoded>>>,
    state: Signal<CaptureState>,
    closed: Cell<bool>,
}

/// A running capture of one output; dropping it stops the capture.
pub struct Capture {
    /// The latest frame as straight opaque RGBA8; `None` until the first one arrives.
    pub frame: Signal<Option<Arc<Decoded>>>,
    /// Where the capture stands.
    pub state: Signal<CaptureState>,
    shared: Rc<Shared>,
}

impl Drop for Capture {
    fn drop(&mut self) {
        self.shared.closed.set(true);
    }
}

pub(crate) struct JobData(pub u64);

pub(crate) struct ManagerData;

pub(crate) struct BufferData;

impl Dispatch2<WlBuffer, State> for BufferData {
    fn event(
        &self,
        _: &mut State,
        _: &WlBuffer,
        _: wayland_client::protocol::wl_buffer::Event,
        _: &Connection,
        _: &QueueHandle<State>,
    ) {
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Layout {
    pub width: u32,
    pub height: u32,
    pub stride: u32,
    pub format: PixelFormat,
}

struct Slab {
    pool: RawPool,
    buffer: WlBuffer,
    layout: Layout,
}

enum Backend {
    Ext(ext::Session),
    Wlr(wlr::Session),
}

pub(crate) struct Job {
    shared: Rc<Shared>,
    pacer: Pacer,
    started: Instant,
    output: WlOutput,
    cursor: bool,
    backend: Backend,
    slab: Option<Slab>,
}

impl Drop for Job {
    fn drop(&mut self) {
        match &mut self.backend {
            Backend::Ext(session) => session.destroy(),
            Backend::Wlr(session) => session.destroy(),
        }
        if let Some(slab) = self.slab.take() {
            slab.buffer.destroy();
        }
    }
}

enum Globals {
    Ext(ext::Globals),
    Wlr(wlr::Globals),
    None,
}

pub(crate) struct Captures {
    globals: Globals,
    jobs: HashMap<u64, Job>,
    next: u64,
}

impl Captures {
    pub fn bind(globals: &GlobalList, qh: &QueueHandle<State>) -> Captures {
        let globals = if let Some(ext) = ext::Globals::bind(globals, qh) {
            Globals::Ext(ext)
        } else if let Some(wlr) = wlr::Globals::bind(globals, qh) {
            Globals::Wlr(wlr)
        } else {
            Globals::None
        };
        Captures { globals, jobs: HashMap::new(), next: 1 }
    }

    pub fn is_supported(&self) -> bool {
        !matches!(self.globals, Globals::None)
    }

    pub fn is_empty(&self) -> bool {
        self.jobs.is_empty()
    }
}

pub(crate) fn start(state: &mut State, output: WlOutput, config: CaptureConfig) -> Result<Capture, CaptureError> {
    let id = state.captures.next;
    let qh = state.qh.clone();
    let backend = match &state.captures.globals {
        Globals::Ext(globals) => Backend::Ext(ext::start(globals, &output, config.cursor, id, &qh)),
        Globals::Wlr(globals) => Backend::Wlr(wlr::Session::new(globals.clone())),
        Globals::None => return Err(CaptureError::Unsupported),
    };
    state.captures.next += 1;
    let shared = Rc::new(Shared {
        frame: Signal::new(None),
        state: Signal::new(CaptureState::Starting),
        closed: Cell::new(false),
    });
    let job = Job {
        shared: shared.clone(),
        pacer: Pacer::new(config.live, config.max_fps),
        started: Instant::now(),
        output,
        cursor: config.cursor,
        backend,
        slab: None,
    };
    let wlr = matches!(job.backend, Backend::Wlr(_));
    state.captures.jobs.insert(id, job);
    if wlr {
        request(state, id);
    }
    Ok(Capture { frame: shared.frame.clone(), state: shared.state.clone(), shared })
}

pub(crate) fn sweep(state: &mut State) {
    state.captures.jobs.retain(|_, job| !job.shared.closed.get());
}

pub(crate) fn request(state: &mut State, id: u64) {
    let qh = state.qh.clone();
    let Some(job) = state.captures.jobs.get_mut(&id) else { return };
    if job.shared.closed.get() {
        state.captures.jobs.remove(&id);
        return;
    }
    job.started = Instant::now();
    match &mut job.backend {
        Backend::Ext(_) => ext::request(state, id),
        Backend::Wlr(session) => session.request(&job.output, job.cursor, id, &qh),
    }
}

pub(crate) fn allocate(state: &mut State, id: u64, layout: Layout) -> Option<WlBuffer> {
    let qh = state.qh.clone();
    let job = state.captures.jobs.get_mut(&id)?;
    if let Some(slab) = job.slab.as_ref().filter(|s| s.layout == layout) {
        return Some(slab.buffer.clone());
    }
    if let Some(old) = job.slab.take() {
        old.buffer.destroy();
    }
    let size = layout.stride as usize * layout.height as usize;
    let mut pool = match RawPool::new(size, &state.shm) {
        Ok(pool) => pool,
        Err(e) => {
            tracing::warn!("cannot allocate a capture buffer: {e}");
            return None;
        }
    };
    let buffer = pool.create_buffer(
        0,
        layout.width as i32,
        layout.height as i32,
        layout.stride as i32,
        layout.format.to_wl(),
        BufferData,
        &qh,
    );
    job.slab = Some(Slab { pool, buffer: buffer.clone(), layout });
    Some(buffer)
}

fn schedule(state: &mut State, id: u64, delay: Duration) {
    let timer = state.loop_handle.insert_source(Timer::from_duration(delay), move |_, _, state| {
        request(state, id);
        TimeoutAction::Drop
    });
    if let Err(e) = timer {
        tracing::warn!("cannot schedule the next capture: {}", e.error);
        fatal(state, id, "cannot schedule the next frame");
    }
}

pub(crate) fn complete(state: &mut State, id: u64, y_invert: bool) {
    let Some(job) = state.captures.jobs.get_mut(&id) else { return };
    let Some(slab) = job.slab.as_mut() else {
        return failed(state, id);
    };
    let layout = slab.layout;
    let bytes = layout.stride as usize * layout.height as usize;
    let rgba = to_rgba(layout.format, layout.width, layout.height, layout.stride, &slab.pool.mmap()[..bytes], y_invert);
    let Some(rgba) = rgba else {
        return failed(state, id);
    };
    let frame = Arc::new(Decoded { width: layout.width, height: layout.height, rgba, animation: None });
    let next = job.pacer.after_frame(job.started, Instant::now());
    match &mut job.backend {
        Backend::Ext(session) => session.release_frame(),
        Backend::Wlr(session) => session.release_frame(),
    }
    let shared = job.shared.clone();
    batch(|| {
        shared.frame.set(Some(frame));
        shared.state.set(if next.is_some() { CaptureState::Streaming } else { CaptureState::Done });
    });
    match next {
        Some(delay) => schedule(state, id, delay),
        None => {
            state.captures.jobs.remove(&id);
        }
    }
}

pub(crate) fn failed(state: &mut State, id: u64) {
    let Some(job) = state.captures.jobs.get_mut(&id) else { return };
    match &mut job.backend {
        Backend::Ext(session) => session.release_frame(),
        Backend::Wlr(session) => session.release_frame(),
    }
    match job.pacer.after_failure() {
        Some(delay) => schedule(state, id, delay),
        None => fatal(state, id, "the compositor kept refusing the capture"),
    }
}

pub(crate) fn fatal(state: &mut State, id: u64, reason: &str) {
    if let Some(job) = state.captures.jobs.remove(&id) {
        job.shared.state.set(CaptureState::Failed(reason.to_owned()));
    }
}

pub(crate) fn session_mut(state: &mut State, id: u64) -> Option<&mut ext::Session> {
    match &mut state.captures.jobs.get_mut(&id)?.backend {
        Backend::Ext(session) => Some(session),
        Backend::Wlr(_) => None,
    }
}

pub(crate) fn wlr_session_mut(state: &mut State, id: u64) -> Option<&mut wlr::Session> {
    match &mut state.captures.jobs.get_mut(&id)?.backend {
        Backend::Wlr(session) => Some(session),
        Backend::Ext(_) => None,
    }
}
