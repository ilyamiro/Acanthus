use smithay_client_toolkit::dispatch2::Dispatch2;
use wayland_client::{Connection, QueueHandle};
use wayland_protocols::wp::idle_inhibit::zv1::client::zwp_idle_inhibit_manager_v1::ZwpIdleInhibitManagerV1;
use wayland_protocols::wp::idle_inhibit::zv1::client::zwp_idle_inhibitor_v1::ZwpIdleInhibitorV1;

use crate::app::State;

pub(crate) struct IdleInhibitGlobal;

impl Dispatch2<ZwpIdleInhibitManagerV1, State> for IdleInhibitGlobal {
    fn event(
        &self,
        _: &mut State,
        _: &ZwpIdleInhibitManagerV1,
        _: <ZwpIdleInhibitManagerV1 as wayland_client::Proxy>::Event,
        _: &Connection,
        _: &QueueHandle<State>,
    ) {
    }
}

impl Dispatch2<ZwpIdleInhibitorV1, State> for IdleInhibitGlobal {
    fn event(
        &self,
        _: &mut State,
        _: &ZwpIdleInhibitorV1,
        _: <ZwpIdleInhibitorV1 as wayland_client::Proxy>::Event,
        _: &Connection,
        _: &QueueHandle<State>,
    ) {
    }
}
