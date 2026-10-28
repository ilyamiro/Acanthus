use smithay_client_toolkit::dispatch2::Dispatch2;
use wayland_client::globals::GlobalList;
use wayland_client::protocol::wl_output::WlOutput;
use wayland_client::protocol::wl_shm::Format;
use wayland_client::{Connection, Proxy, QueueHandle, WEnum};
use wayland_protocols::ext::image_capture_source::v1::client::ext_output_image_capture_source_manager_v1::ExtOutputImageCaptureSourceManagerV1 as SourceManager;
use wayland_protocols::ext::image_copy_capture::v1::client::ext_image_copy_capture_frame_v1::{
    self as frame_events, ExtImageCopyCaptureFrameV1 as Frame,
};
use wayland_protocols::ext::image_copy_capture::v1::client::ext_image_copy_capture_manager_v1::{
    ExtImageCopyCaptureManagerV1 as CopyManager, Options,
};
use wayland_protocols::ext::image_copy_capture::v1::client::ext_image_copy_capture_session_v1::{
    self as session_events, ExtImageCopyCaptureSessionV1 as SessionProxy,
};

use super::pixels::pick;
use super::{JobData, Layout, ManagerData, allocate, complete, failed, fatal, request as request_frame};
use crate::app::State;

pub(super) struct Globals {
    copy: CopyManager,
    source: SourceManager,
}

impl Globals {
    pub fn bind(globals: &GlobalList, qh: &QueueHandle<State>) -> Option<Globals> {
        Some(Globals {
            copy: globals.bind(qh, 1..=1, ManagerData).ok()?,
            source: globals.bind(qh, 1..=1, ManagerData).ok()?,
        })
    }
}

pub(crate) struct Session {
    session: SessionProxy,
    frame: Option<Frame>,
    formats: Vec<Format>,
    size: Option<(u32, u32)>,
    batch_done: bool,
    constraints_ready: bool,
    want_frame: bool,
}

impl Session {
    pub fn destroy(&mut self) {
        self.release_frame();
        if self.session.is_alive() {
            self.session.destroy();
        }
    }

    pub fn release_frame(&mut self) {
        if let Some(frame) = self.frame.take()
            && frame.is_alive()
        {
            frame.destroy();
        }
    }

    fn restart_batch(&mut self) {
        if self.batch_done {
            self.formats.clear();
            self.batch_done = false;
            self.constraints_ready = false;
        }
    }
}

pub(super) fn start(globals: &Globals, output: &WlOutput, cursor: bool, id: u64, qh: &QueueHandle<State>) -> Session {
    let source = globals.source.create_source(output, qh, ManagerData);
    let options = if cursor { Options::PaintCursors } else { Options::empty() };
    let session = globals.copy.create_session(&source, options, qh, JobData(id));
    source.destroy();
    Session {
        session,
        frame: None,
        formats: Vec::new(),
        size: None,
        batch_done: false,
        constraints_ready: false,
        want_frame: true,
    }
}

pub(super) fn request(state: &mut State, id: u64) {
    let qh = state.qh.clone();
    let Some(session) = super::session_mut(state, id) else { return };
    if session.frame.is_some() {
        return;
    }
    if !session.constraints_ready {
        session.want_frame = true;
        return;
    }
    let Some((width, height)) = session.size else { return };
    let Some(format) = pick(&session.formats) else {
        return fatal(state, id, "the compositor offers no shared-memory format this engine reads");
    };
    let layout = Layout { width, height, stride: width * 4, format };
    let Some(buffer) = allocate(state, id, layout) else {
        return failed(state, id);
    };
    let Some(session) = super::session_mut(state, id) else { return };
    let frame = session.session.create_frame(&qh, JobData(id));
    frame.attach_buffer(&buffer);
    frame.damage_buffer(0, 0, width as i32, height as i32);
    frame.capture();
    session.frame = Some(frame);
}

impl Dispatch2<CopyManager, State> for ManagerData {
    fn event(
        &self,
        _: &mut State,
        _: &CopyManager,
        _: <CopyManager as Proxy>::Event,
        _: &Connection,
        _: &QueueHandle<State>,
    ) {
    }
}

impl Dispatch2<SourceManager, State> for ManagerData {
    fn event(
        &self,
        _: &mut State,
        _: &SourceManager,
        _: <SourceManager as Proxy>::Event,
        _: &Connection,
        _: &QueueHandle<State>,
    ) {
    }
}

impl
    Dispatch2<
        wayland_protocols::ext::image_capture_source::v1::client::ext_image_capture_source_v1::ExtImageCaptureSourceV1,
        State,
    > for ManagerData
{
    fn event(
        &self,
        _: &mut State,
        _: &wayland_protocols::ext::image_capture_source::v1::client::ext_image_capture_source_v1::ExtImageCaptureSourceV1,
        _: <wayland_protocols::ext::image_capture_source::v1::client::ext_image_capture_source_v1::ExtImageCaptureSourceV1 as Proxy>::Event,
        _: &Connection,
        _: &QueueHandle<State>,
    ) {
    }
}

impl Dispatch2<SessionProxy, State> for JobData {
    fn event(
        &self,
        state: &mut State,
        _: &SessionProxy,
        event: session_events::Event,
        _: &Connection,
        _: &QueueHandle<State>,
    ) {
        let id = self.0;
        match event {
            session_events::Event::BufferSize { width, height } => {
                if let Some(session) = super::session_mut(state, id) {
                    session.restart_batch();
                    session.size = Some((width, height));
                }
            }
            session_events::Event::ShmFormat { format } => {
                if let Some(session) = super::session_mut(state, id) {
                    session.restart_batch();
                    if let WEnum::Value(format) = format {
                        session.formats.push(format);
                    }
                }
            }
            session_events::Event::Done => {
                let Some(session) = super::session_mut(state, id) else { return };
                session.batch_done = true;
                session.constraints_ready = true;
                if std::mem::take(&mut session.want_frame) {
                    request_frame(state, id);
                }
            }
            session_events::Event::Stopped => fatal(state, id, "the capture session was stopped by the compositor"),
            _ => {}
        }
    }
}

impl Dispatch2<Frame, State> for JobData {
    fn event(&self, state: &mut State, _: &Frame, event: frame_events::Event, _: &Connection, _: &QueueHandle<State>) {
        let id = self.0;
        match event {
            frame_events::Event::Ready => complete(state, id, false),
            frame_events::Event::Failed { reason } => {
                if matches!(reason, WEnum::Value(frame_events::FailureReason::Stopped)) {
                    fatal(state, id, "the capture source is gone");
                } else {
                    failed(state, id);
                }
            }
            _ => {}
        }
    }
}
