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
    use std::cell::Cell;

    use acanthus_scene::{Fonts, MouseButton};

    use super::*;

    struct Fixture {
        scene: Scene,
        keep: Keep,
        alive: Signal<bool>,
        closed: Rc<Cell<u32>>,
        popup_clicks: Rc<Cell<u32>>,
    }

    fn fixture() -> Fixture {
        let mut scene = Scene::new(Fonts::new());
        scene.set_viewport(Vec2::new(400.0, 300.0), 1.0);
        let mut keep = Keep::default();
        let alive = Signal::new(true);
        let closed = Rc::new(Cell::new(0));
        let popup_clicks = Rc::new(Cell::new(0));
        let root = scene.root();
        let viewport = scene.viewport();
        let c = closed.clone();
        let spec = Spec {
            alive: alive.clone(),
            popup: Rc::new(|| Rect::new(100.0, 100.0, 100.0, 100.0)),
            reserved: Rc::new(|size| Rect::new(0.0, 0.0, size.x, 40.0)),
            close: Rc::new(move || c.set(c.get() + 1)),
        };
        attach(&mut scene, root, &mut keep, &viewport, spec);
        let p = popup_clicks.clone();
        let area = MouseArea::new().on_click(move |_| p.set(p.get() + 1));
        scene.add(root, Node::rect().x(100.0).y(100.0).size(100.0, 100.0).mouse(area));
        scene.frame();
        Fixture { scene, keep, alive, closed, popup_clicks }
    }

    fn click(scene: &mut Scene, x: f32, y: f32) {
        scene.pointer_motion(Vec2::new(x, y));
        scene.pointer_button(MouseButton::Left, true);
        scene.pointer_button(MouseButton::Left, false);
    }

    #[test]
    fn a_click_outside_the_popup_closes_and_one_inside_does_not() {
        let mut f = fixture();
        click(&mut f.scene, 300.0, 250.0);
        assert_eq!(f.closed.get(), 1);
        click(&mut f.scene, 150.0, 150.0);
        assert_eq!((f.closed.get(), f.popup_clicks.get()), (1, 1));
        drop(f.keep);
    }

    #[test]
    fn the_input_region_follows_alive_and_the_viewport() {
        let mut f = fixture();
        let region = f.scene.input_region();
        assert_eq!(
            region.get_untracked(),
            Some(vec![Rect::new(0.0, 40.0, 400.0, 260.0), Rect::new(100.0, 100.0, 100.0, 100.0)])
        );
        f.alive.set(false);
        assert_eq!(region.get_untracked(), Some(Vec::new()));
        f.alive.set(true);
        f.scene.set_viewport(Vec2::new(800.0, 600.0), 1.0);
        assert_eq!(region.get_untracked().unwrap()[0], Rect::new(0.0, 40.0, 800.0, 560.0));
    }

    #[test]
    fn a_hidden_layer_ignores_clicks() {
        let mut f = fixture();
        f.alive.set(false);
        f.scene.frame();
        click(&mut f.scene, 300.0, 250.0);
        assert_eq!(f.closed.get(), 0);
    }

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
