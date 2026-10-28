//! Click-outside dismissal for popups that live on a full-output overlay surface.
//!
//! The overlay only receives input where something needs it: the whole surface minus a reserved
//! strip (for example a bar the popup must not cover), plus the popup itself. A transparent layer
//! under the popup closes it on any click that reaches it.

use std::rc::Rc;

use acanthus_anim::Keep;
use acanthus_core::{Rect, Signal, Vec2};
use acanthus_scene::{Anchors, MouseArea, Node, NodeId, Scene};

/// What a popup surface needs to dismiss itself on a click outside.
pub struct Spec {
    /// Whether the popup is shown; when false the input region is empty and the layer is hidden.
    pub alive: Signal<bool>,
    /// Bounds of the popup in surface coordinates; clicks inside it never dismiss.
    pub popup: Rc<dyn Fn() -> Rect>,
    /// Strip of the surface that must stay click-through, given the surface size. A rectangle
    /// with no area means nothing is reserved.
    pub reserved: Rc<dyn Fn(Vec2) -> Rect>,
    /// Called when the transparent layer is clicked.
    pub close: Rc<dyn Fn()>,
}

/// The part of a surface of `size` left over once the edge strip `reserved` is cut away.
///
/// A strip touching a full edge removes that band; any other rectangle keeps the side that starts
/// at the origin. A rectangle with no area leaves the whole surface.
pub fn free_area(reserved: Rect, size: Vec2) -> Rect {
    let Rect { x: hx, y: hy, w: hw, h: hh } = reserved;
    if hw <= 0.0 || hh <= 0.0 {
        Rect::new(0.0, 0.0, size.x, size.y)
    } else if hw >= size.x && hy <= 0.0 {
        Rect::new(0.0, hh, size.x, size.y - hh)
    } else if hw >= size.x {
        Rect::new(0.0, 0.0, size.x, hy)
    } else if hx <= 0.0 {
        Rect::new(hw, 0.0, size.x - hw, size.y)
    } else {
        Rect::new(0.0, 0.0, hx, size.y)
    }
}

/// Input regions of a popup surface: the free area plus the popup bounds.
pub fn regions(reserved: Rect, size: Vec2, popup: Rect) -> Vec<Rect> {
    vec![free_area(reserved, size), popup]
}

/// Keeps `scene`'s input region equal to the free area (plus `popup` when given) while `alive`,
/// and empty otherwise. The binding lives as long as `keep`.
pub fn bind_input_region(
    scene: &Scene,
    keep: &mut Keep,
    viewport: &Signal<Vec2>,
    alive: &Signal<bool>,
    reserved: Rc<dyn Fn(Vec2) -> Rect>,
    popup: Option<Rc<dyn Fn() -> Rect>>,
) {
    let (region, alive, viewport) = (scene.input_region(), alive.clone(), viewport.clone());
    keep.add(Signal::new(()).bind(move || {
        if !alive.get() {
            region.set(Some(Vec::new()));
            return;
        }
        let size = viewport.get();
        let free = free_area(reserved(size), size);
        region.set(Some(match &popup {
            Some(popup) => vec![free, popup()],
            None => vec![free],
        }));
    }));
}

/// Binds the input region from `spec` and adds the transparent click layer under `parent`.
/// Returns the layer; add the popup afterwards so it paints and receives input above it.
pub fn attach(scene: &mut Scene, parent: NodeId, keep: &mut Keep, viewport: &Signal<Vec2>, spec: Spec) -> NodeId {
    let Spec { alive, popup, reserved, close } = spec;
    bind_input_region(scene, keep, viewport, &alive, reserved, Some(popup));
    let area = MouseArea::new().on_click(move |_| close());
    scene.add(parent, Node::item().anchors(Anchors::fill(0.0)).visible(&alive).mouse(area))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn size() -> Vec2 {
        Vec2::new(1920.0, 1080.0)
    }

    #[test]
    fn nothing_reserved_leaves_the_whole_surface() {
        assert_eq!(free_area(Rect::new(0.0, 0.0, 0.0, 0.0), size()), Rect::new(0.0, 0.0, 1920.0, 1080.0));
    }

    #[test]
    fn a_strip_on_each_edge_is_cut_away() {
        assert_eq!(free_area(Rect::new(0.0, 0.0, 1920.0, 48.0), size()), Rect::new(0.0, 48.0, 1920.0, 1032.0));
        assert_eq!(free_area(Rect::new(0.0, 1032.0, 1920.0, 48.0), size()), Rect::new(0.0, 0.0, 1920.0, 1032.0));
        assert_eq!(free_area(Rect::new(0.0, 0.0, 48.0, 1080.0), size()), Rect::new(48.0, 0.0, 1872.0, 1080.0));
        assert_eq!(free_area(Rect::new(1872.0, 0.0, 48.0, 1080.0), size()), Rect::new(0.0, 0.0, 1872.0, 1080.0));
    }

    #[test]
    fn the_regions_are_the_free_area_plus_the_popup() {
        let popup = Rect::new(100.0, 60.0, 300.0, 200.0);
        let top = regions(Rect::new(0.0, 0.0, 1920.0, 48.0), size(), popup);
        assert_eq!(top, vec![Rect::new(0.0, 48.0, 1920.0, 1032.0), popup]);
    }
}
