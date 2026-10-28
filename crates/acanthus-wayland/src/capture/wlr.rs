use smithay_client_toolkit::dispatch2::Dispatch2;
use wayland_client::globals::GlobalList;
use wayland_client::protocol::wl_output::WlOutput;
use wayland_client::{Connection, Proxy, QueueHandle, WEnum};
use wayland_protocols_wlr::screencopy::v1::client::zwlr_screencopy_frame_v1::{
    self as frame_events, Flags, ZwlrScreencopyFrameV1 as Frame,
};
use wayland_protocols_wlr::screencopy::v1::client::zwlr_screencopy_manager_v1::ZwlrScreencopyManagerV1 as Manager;

use super::pixels::PixelFormat;
use super::{JobData, Layout, ManagerData, allocate, complete, failed};
use crate::app::State;

#[derive(Clone)]
pub(super) struct Globals {
    manager: Manager,
}

impl Globals {
    pub fn bind(globals: &GlobalList, qh: &QueueHandle<State>) -> Option<Globals> {
        Some(Globals { manager: globals.bind(qh, 1..=3, ManagerData).ok()? })
    }
}

pub(crate) struct Session {
    globals: Globals,
    frame: Option<Frame>,
    layout: Option<Layout>,
    y_invert: bool,
    copied: bool,
}

impl Session {
    pub(super) fn new(globals: Globals) -> Session {
        Session { globals, frame: None, layout: None, y_invert: false, copied: false }
    }

    pub fn destroy(&mut self) {
        self.release_frame();
    }

    pub fn release_frame(&mut self) {
        if let Some(frame) = self.frame.take()
            && frame.is_alive()
        {
            frame.destroy();
        }
    }

    pub fn request(&mut self, output: &WlOutput, cursor: bool, id: u64, qh: &QueueHandle<State>) {
        if self.frame.is_some() {
            return;
        }
        self.layout = None;
        self.y_invert = false;
        self.copied = false;
        self.frame = Some(self.globals.manager.capture_output(i32::from(cursor), output, qh, JobData(id)));
    }
}

impl Dispatch2<Manager, State> for ManagerData {
    fn event(&self, _: &mut State, _: &Manager, _: <Manager as Proxy>::Event, _: &Connection, _: &QueueHandle<State>) {}
}

fn copy_now(state: &mut State, id: u64, frame: &Frame) {
    let Some(session) = super::wlr_session_mut(state, id) else { return };
    if std::mem::replace(&mut session.copied, true) {
        return;
    }
    let Some(layout) = session.layout else {
        return failed(state, id);
    };
    match allocate(state, id, layout) {
        Some(buffer) => frame.copy(&buffer),
        None => failed(state, id),
    }
}

impl Dispatch2<Frame, State> for JobData {
    fn event(
        &self,
        state: &mut State,
        frame: &Frame,
        event: frame_events::Event,
        _: &Connection,
        _: &QueueHandle<State>,
    ) {
        let id = self.0;
        match event {
            frame_events::Event::Buffer { format, width, height, stride } => {
                let Some(session) = super::wlr_session_mut(state, id) else { return };
                if let WEnum::Value(format) = format
                    && let Some(format) = PixelFormat::from_wl(format)
                {
                    session.layout = Some(Layout { width, height, stride, format });
                }
                if frame.version() < 3 {
                    copy_now(state, id, frame);
                }
            }
            frame_events::Event::BufferDone => copy_now(state, id, frame),
            frame_events::Event::Flags { flags } => {
                if let Some(session) = super::wlr_session_mut(state, id) {
                    session.y_invert = matches!(flags, WEnum::Value(f) if f.contains(Flags::YInvert));
                }
            }
            frame_events::Event::Ready { .. } => {
                let y_invert = super::wlr_session_mut(state, id).is_some_and(|s| s.y_invert);
                complete(state, id, y_invert);
            }
            frame_events::Event::Failed => failed(state, id),
            _ => {}
        }
    }
}
