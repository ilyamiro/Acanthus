use smithay_client_toolkit::dispatch2::Dispatch2;
use smithay_client_toolkit::reexports::protocols::wp::fractional_scale::v1::client::{
    wp_fractional_scale_manager_v1::WpFractionalScaleManagerV1,
    wp_fractional_scale_v1::{self, WpFractionalScaleV1},
};
use smithay_client_toolkit::reexports::protocols::wp::viewporter::client::{
    wp_viewport::WpViewport, wp_viewporter::WpViewporter,
};
use wayland_client::protocol::wl_surface::WlSurface;
use wayland_client::{Connection, QueueHandle};

use crate::app::State;

pub(crate) const SCALE_DENOMINATOR: f32 = 120.0;

pub(crate) struct ScaleGlobal;

pub(crate) struct FractionalData {
    pub surface: WlSurface,
}

pub(crate) struct ScaleManagers {
    pub fractional: WpFractionalScaleManagerV1,
    pub viewporter: WpViewporter,
}

pub(crate) struct ScaleObjects {
    pub fractional: WpFractionalScaleV1,
    pub viewport: WpViewport,
}

impl Drop for ScaleObjects {
    fn drop(&mut self) {
        self.viewport.destroy();
        self.fractional.destroy();
    }
}

pub(crate) fn scale_from_wire(value: u32) -> f32 {
    value as f32 / SCALE_DENOMINATOR
}

pub(crate) fn buffer_size(logical: (u32, u32), scale: f32) -> (u32, u32) {
    let px = |v: u32| ((v.max(1) as f32 * scale).round() as u32).max(1);
    (px(logical.0), px(logical.1))
}

impl Dispatch2<WpFractionalScaleManagerV1, State> for ScaleGlobal {
    fn event(
        &self,
        _: &mut State,
        _: &WpFractionalScaleManagerV1,
        _: <WpFractionalScaleManagerV1 as wayland_client::Proxy>::Event,
        _: &Connection,
        _: &QueueHandle<State>,
    ) {
    }
}

impl Dispatch2<WpViewporter, State> for ScaleGlobal {
    fn event(
        &self,
        _: &mut State,
        _: &WpViewporter,
        _: <WpViewporter as wayland_client::Proxy>::Event,
        _: &Connection,
        _: &QueueHandle<State>,
    ) {
    }
}

impl Dispatch2<WpViewport, State> for ScaleGlobal {
    fn event(
        &self,
        _: &mut State,
        _: &WpViewport,
        _: <WpViewport as wayland_client::Proxy>::Event,
        _: &Connection,
        _: &QueueHandle<State>,
    ) {
    }
}

impl Dispatch2<WpFractionalScaleV1, State> for FractionalData {
    fn event(
        &self,
        state: &mut State,
        _: &WpFractionalScaleV1,
        event: wp_fractional_scale_v1::Event,
        _: &Connection,
        _: &QueueHandle<State>,
    ) {
        if let wp_fractional_scale_v1::Event::PreferredScale { scale } = event {
            state.set_fractional_scale(&self.surface, scale_from_wire(scale));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wire_scale_is_in_one_hundred_twentieths() {
        assert_eq!(scale_from_wire(120), 1.0);
        assert_eq!(scale_from_wire(150), 1.25);
        assert_eq!(scale_from_wire(180), 1.5);
    }

    #[test]
    fn buffer_size_rounds_to_the_nearest_pixel() {
        assert_eq!(buffer_size((100, 40), 1.25), (125, 50));
        assert_eq!(buffer_size((101, 33), 1.5), (152, 50));
        assert_eq!(buffer_size((0, 0), 2.0), (2, 2));
    }
}
