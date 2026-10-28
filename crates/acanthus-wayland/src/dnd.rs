use std::collections::HashMap;
use std::fs::File;
use std::io::{self, Read, Write};
use std::os::fd::AsFd;
use std::sync::{Arc, Mutex};

use acanthus_core::{Signal, Vec2};
use acanthus_scene::{DragData, DragOffer, DropAction, DropActions};
use calloop::channel::{Event as ChannelEvent, channel};
use smithay_client_toolkit::dispatch2::Dispatch2;
use wayland_client::globals::GlobalList;
use wayland_client::protocol::wl_data_device::{self, WlDataDevice};
use wayland_client::protocol::wl_data_device_manager::{DndAction, WlDataDeviceManager};
use wayland_client::protocol::wl_data_offer::{self, WlDataOffer};
use wayland_client::protocol::wl_data_source::{self, WlDataSource};
use wayland_client::protocol::wl_seat::WlSeat;
use wayland_client::{Connection, Proxy, QueueHandle, WEnum};

use crate::app::{State, SurfaceId};

const MAX_BYTES: u64 = 16 * 1024 * 1024;
const MAX_MIMES: usize = 8;
const PREFERRED_MIMES: [&str; 3] = ["text/uri-list", "text/plain;charset=utf-8", "text/plain"];

/// How a drag started with [`crate::AppHandle::start_drag`] ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DragOutcome {
    /// The drag is running.
    Running,
    /// A target accepted the drop and performed this action.
    Dropped(DropAction),
    /// The drag was cancelled or dropped where nothing accepts it.
    Cancelled,
}

pub(crate) struct DragRequest {
    pub surface: SurfaceId,
    pub data: DragData,
    pub actions: DropActions,
    pub outcome: Signal<DragOutcome>,
}

pub(crate) struct ManagerData;

#[derive(Default)]
pub(crate) struct OfferData {
    mimes: Mutex<Vec<String>>,
    actions: Mutex<u32>,
}

pub(crate) struct DeviceData;

pub(crate) struct SourceData {
    id: u64,
    entries: Arc<Vec<(String, Vec<u8>)>>,
}

struct Target {
    surface: SurfaceId,
    offer: WlDataOffer,
    serial: u32,
    mimes: Vec<String>,
}

pub(crate) struct Dnd {
    manager: Option<WlDataDeviceManager>,
    device: Option<WlDataDevice>,
    target: Option<Target>,
    sources: HashMap<u64, (WlDataSource, Signal<DragOutcome>, Option<DropAction>)>,
    next: u64,
}

pub(crate) fn bits(actions: DropActions) -> DndAction {
    let mut out = DndAction::empty();
    if actions.copy {
        out |= DndAction::Copy;
    }
    if actions.move_ {
        out |= DndAction::Move;
    }
    out
}

pub(crate) fn from_bits(actions: DndAction) -> DropActions {
    DropActions { copy: actions.contains(DndAction::Copy), move_: actions.contains(DndAction::Move), link: false }
}

pub(crate) fn action_bits(action: DropAction) -> DndAction {
    match action {
        DropAction::Copy => DndAction::Copy,
        DropAction::Move => DndAction::Move,
        DropAction::Link => DndAction::empty(),
    }
}

pub(crate) fn action_from(bits: DndAction) -> Option<DropAction> {
    if bits.contains(DndAction::Copy) {
        Some(DropAction::Copy)
    } else if bits.contains(DndAction::Move) {
        Some(DropAction::Move)
    } else {
        None
    }
}

pub(crate) fn mimes_to_read(offered: &[String]) -> Vec<String> {
    let mut chosen: Vec<String> =
        PREFERRED_MIMES.iter().filter(|m| offered.iter().any(|o| o == *m)).map(|m| (*m).to_owned()).collect();
    for mime in offered {
        if chosen.len() >= MAX_MIMES {
            break;
        }
        if !chosen.contains(mime) && !mime.starts_with("application/x-kde") {
            chosen.push(mime.clone());
        }
    }
    chosen
}

impl Dnd {
    pub fn bind(globals: &GlobalList, qh: &QueueHandle<State>) -> Dnd {
        Dnd {
            manager: globals.bind(qh, 1..=3, ManagerData).ok(),
            device: None,
            target: None,
            sources: HashMap::new(),
            next: 1,
        }
    }

    pub fn ensure_device(&mut self, seat: &WlSeat, qh: &QueueHandle<State>) {
        if self.device.is_none()
            && let Some(manager) = &self.manager
        {
            self.device = Some(manager.get_data_device(seat, qh, DeviceData));
        }
    }
}

pub(crate) fn start_drag(state: &mut State, request: DragRequest) {
    let qh = state.qh.clone();
    let serial = state.seats.first().zip(state.input_serial).map(|(_, serial)| serial);
    let Some(origin) = state.surfaces.get(request.surface).map(|s| s.wl_surface().clone()) else {
        return request.outcome.set(DragOutcome::Cancelled);
    };
    let (Some(manager), Some(device), Some(serial)) = (state.dnd.manager.clone(), state.dnd.device.clone(), serial)
    else {
        return request.outcome.set(DragOutcome::Cancelled);
    };
    let id = state.dnd.next;
    state.dnd.next += 1;
    let entries: Vec<(String, Vec<u8>)> = request
        .data
        .mime_types()
        .into_iter()
        .filter_map(|mime| request.data.get(&mime).map(|bytes| (mime.clone(), bytes.to_vec())))
        .collect();
    let source = manager.create_data_source(&qh, SourceData { id, entries: Arc::new(entries.clone()) });
    for (mime, _) in &entries {
        source.offer(mime.clone());
    }
    if source.version() >= 3 {
        source.set_actions(bits(request.actions));
    }
    device.start_drag(Some(&source), &origin, None, serial);
    request.outcome.set(DragOutcome::Running);
    state.dnd.sources.insert(id, (source, request.outcome, None));
}

fn finish_source(state: &mut State, id: u64, outcome: DragOutcome) {
    if let Some((source, signal, _)) = state.dnd.sources.remove(&id) {
        signal.set(outcome);
        if source.is_alive() {
            source.destroy();
        }
    }
}

fn last_action(state: &State, id: u64) -> Option<DropAction> {
    state.dnd.sources.get(&id).and_then(|(_, _, action)| *action)
}

fn set_action(state: &mut State, id: u64, action: Option<DropAction>) {
    if let Some(entry) = state.dnd.sources.get_mut(&id) {
        entry.2 = action;
    }
}

fn answer(state: &mut State, action: Option<DropAction>) {
    let Some(target) = state.dnd.target.as_ref() else { return };
    let mime = target.mimes.first().cloned();
    match action {
        Some(action) => {
            target.offer.accept(target.serial, mime);
            if target.offer.version() >= 3 {
                let bits = action_bits(action);
                target.offer.set_actions(bits, bits);
            }
        }
        None => target.offer.accept(target.serial, None),
    }
}

fn drop_target(state: &mut State, conn: &Connection) {
    let Some(target) = state.dnd.target.take() else { return };
    let wanted = mimes_to_read(&target.mimes);
    let mut pipes = Vec::new();
    for mime in wanted {
        let Ok((reader, writer)) = io::pipe() else { continue };
        target.offer.receive(mime.clone(), writer.as_fd());
        pipes.push((mime, File::from(std::os::fd::OwnedFd::from(reader))));
    }
    let _ = conn.flush();
    let (sender, receiver) = channel::<Vec<(String, Vec<u8>)>>();
    let surface = target.surface;
    let offer = target.offer.clone();
    std::thread::spawn(move || {
        let mut entries = Vec::new();
        for (mime, mut reader) in pipes {
            let mut bytes = Vec::new();
            if Read::take(&mut reader, MAX_BYTES).read_to_end(&mut bytes).is_ok() {
                entries.push((mime, bytes));
            }
        }
        let _ = sender.send(entries);
    });
    let inserted = state.loop_handle.insert_source(receiver, move |event, _, state| {
        let ChannelEvent::Msg(entries) = event else { return };
        let data = entries.into_iter().fold(DragData::new(), |data, (mime, bytes)| data.with(&mime, bytes));
        let performed = state.surfaces.get_mut(surface).and_then(|s| s.scene.drag_drop(data));
        if performed.is_some() && offer.version() >= 3 {
            offer.finish();
        }
        if offer.is_alive() {
            offer.destroy();
        }
    });
    if let Err(e) = inserted {
        tracing::warn!("cannot receive the dropped data: {}", e.error);
    }
}

impl Dispatch2<WlDataDeviceManager, State> for ManagerData {
    fn event(
        &self,
        _: &mut State,
        _: &WlDataDeviceManager,
        _: <WlDataDeviceManager as Proxy>::Event,
        _: &Connection,
        _: &QueueHandle<State>,
    ) {
    }
}

impl Dispatch2<WlDataOffer, State> for OfferData {
    fn event(
        &self,
        _: &mut State,
        _: &WlDataOffer,
        event: wl_data_offer::Event,
        _: &Connection,
        _: &QueueHandle<State>,
    ) {
        match event {
            wl_data_offer::Event::Offer { mime_type } => {
                if let Ok(mut mimes) = self.mimes.lock() {
                    mimes.push(mime_type);
                }
            }
            wl_data_offer::Event::SourceActions { source_actions: WEnum::Value(actions) } => {
                if let Ok(mut stored) = self.actions.lock() {
                    *stored = actions.bits();
                }
            }
            _ => {}
        }
    }
}

impl Dispatch2<WlDataDevice, State> for DeviceData {
    wayland_client::event_created_child!(State, WlDataDevice, [
        0 => (WlDataOffer, OfferData::default())
    ]);

    fn event(
        &self,
        state: &mut State,
        _: &WlDataDevice,
        event: wl_data_device::Event,
        conn: &Connection,
        _: &QueueHandle<State>,
    ) {
        match event {
            wl_data_device::Event::Enter { serial, surface, x, y, id: Some(offer) } => {
                let (mimes, source_actions) = offer
                    .data::<OfferData>()
                    .map(|d| {
                        (
                            d.mimes.lock().map(|m| m.clone()).unwrap_or_default(),
                            d.actions.lock().map(|a| *a).unwrap_or(0),
                        )
                    })
                    .unwrap_or_default();
                let Some(id) = state.surface_by_wl(&surface) else {
                    offer.destroy();
                    return;
                };
                let actions = if source_actions == 0 {
                    DropActions::COPY
                } else {
                    from_bits(DndAction::from_bits_truncate(source_actions))
                };
                let drag = DragOffer { mime_types: mimes.clone(), actions, source: None };
                let pos = Vec2::new(x as f32, y as f32);
                let accepted = state.surfaces[id].scene.drag_enter(drag, pos);
                state.dnd.target = Some(Target { surface: id, offer, serial, mimes });
                let action = accepted.then(|| state.surfaces[id].scene.drag_motion(pos)).flatten();
                answer(state, action);
            }
            wl_data_device::Event::Motion { x, y, .. } => {
                let Some(id) = state.dnd.target.as_ref().map(|t| t.surface) else { return };
                let action =
                    state.surfaces.get_mut(id).and_then(|s| s.scene.drag_motion(Vec2::new(x as f32, y as f32)));
                answer(state, action);
            }
            wl_data_device::Event::Leave => {
                if let Some(target) = state.dnd.target.take() {
                    if let Some(window) = state.surfaces.get_mut(target.surface) {
                        window.scene.drag_leave();
                    }
                    target.offer.destroy();
                }
            }
            wl_data_device::Event::Drop => drop_target(state, conn),
            wl_data_device::Event::Selection { id: Some(offer) } => offer.destroy(),
            _ => {}
        }
    }
}

fn write_in_background(fd: std::os::fd::OwnedFd, bytes: Vec<u8>) {
    std::thread::spawn(move || {
        let _ = File::from(fd).write_all(&bytes);
    });
}

impl Dispatch2<WlDataSource, State> for SourceData {
    fn event(
        &self,
        state: &mut State,
        _: &WlDataSource,
        event: wl_data_source::Event,
        _: &Connection,
        _: &QueueHandle<State>,
    ) {
        match event {
            wl_data_source::Event::Send { mime_type, fd } => {
                if let Some((_, bytes)) = self.entries.iter().find(|(m, _)| *m == mime_type) {
                    write_in_background(fd, bytes.clone());
                }
            }
            wl_data_source::Event::DndFinished => {
                let action = last_action(state, self.id);
                finish_source(state, self.id, action.map_or(DragOutcome::Cancelled, DragOutcome::Dropped));
            }
            wl_data_source::Event::Action { dnd_action: WEnum::Value(action) } => {
                set_action(state, self.id, action_from(action));
            }
            wl_data_source::Event::Cancelled => finish_source(state, self.id, DragOutcome::Cancelled),
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn action_bits_map_both_ways() {
        assert_eq!(bits(DropActions::ALL), DndAction::Copy | DndAction::Move);
        assert_eq!(from_bits(DndAction::Move), DropActions::MOVE);
        assert_eq!(action_from(DndAction::Copy | DndAction::Move), Some(DropAction::Copy));
        assert_eq!(action_from(DndAction::Move), Some(DropAction::Move));
        assert_eq!(action_from(DndAction::empty()), None);
        assert_eq!(action_bits(DropAction::Move), DndAction::Move);
    }

    #[test]
    fn dropped_data_is_read_for_the_preferred_types_first_and_capped() {
        let offered: Vec<String> =
            ["image/png", "text/plain", "text/uri-list", "application/x-kde-cut-selection"].map(String::from).to_vec();
        assert_eq!(mimes_to_read(&offered), ["text/uri-list", "text/plain", "image/png"]);
        let many: Vec<String> = (0..20).map(|n| format!("x/{n}")).collect();
        assert_eq!(mimes_to_read(&many).len(), MAX_MIMES);
    }
}
