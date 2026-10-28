use acanthus_core::{Rect, scaled};

use crate::node::IntoSignal;
use crate::scene::{LayerGeometry, Scene};

impl Scene {
    /// Sets the UI scale: views, layout, bounds, pointer positions and scroll steps are in logical units, and one unit covers `scale` compositor pixels. A bound signal (for example a per-screen setting) re-lays out the scene when it changes; non-positive or non-finite values count as 1. The default is 1.
    pub fn set_ui_scale(&mut self, scale: impl IntoSignal<f32>) {
        self.ui_scale = scale.into_signal();
        self.tracker.mark_dirty();
        self.apply_units();
    }

    /// The UI scale in effect: compositor pixels per logical unit.
    pub fn ui_scale(&self) -> f32 {
        self.units
    }

    /// The input region in compositor logical pixels, converted from the logical units of [`Scene::input_region`]; `None` for the whole surface.
    pub fn surface_input_region(&self) -> Option<Vec<Rect>> {
        let units = self.units;
        self.input_region.get_untracked().map(|rects| rects.into_iter().map(|r| r.scale(units)).collect())
    }

    /// The layer placement in compositor logical pixels, converted from the logical units of [`Scene::layer_geometry`] with the [`scaled`] rounding.
    pub fn surface_layer_geometry(&self) -> Option<LayerGeometry> {
        let units = self.units;
        let px = |v: f32| scaled(v, units);
        self.layer_geometry.get_untracked().map(|g| LayerGeometry {
            size: (px(g.size.0 as f32) as u32, px(g.size.1 as f32) as u32),
            margin: g.margin.map(|m| px(m as f32) as i32),
            exclusive_zone: px(g.exclusive_zone as f32) as i32,
            ..g
        })
    }

    pub(crate) fn apply_units(&mut self) {
        let wanted = self.ui_scale.get_untracked();
        let units = if wanted.is_finite() && wanted > 0.0 { wanted } else { 1.0 };
        let viewport = self.surface_size / units;
        let scale = self.output_scale * units;
        if units == self.units && viewport == self.viewport && scale == self.scale {
            return;
        }
        if let Some(pointer) = self.pointer.as_mut() {
            *pointer *= self.units / units;
        }
        self.units = units;
        self.scale = scale;
        if self.viewport != viewport {
            self.viewport = viewport;
            self.viewport_signal.set(viewport);
        }
        self.tracker.mark_dirty();
    }
}

#[cfg(test)]
mod tests {
    use acanthus_core::{Color, Signal, Vec2};

    use super::*;
    use crate::display::Primitive;
    use crate::{Anchors, Layout, MouseArea, Node, ScrollDelta};
    use crate::{Fonts, MouseButton};

    fn quads(s: &mut Scene) -> Vec<Rect> {
        s.frame()
            .primitives
            .iter()
            .filter_map(|p| match p {
                Primitive::Quad(q) => Some(q.rect),
                _ => None,
            })
            .collect()
    }

    fn build(s: &mut Scene, k: f32) {
        let root = s.root();
        let bar = s.add(
            root,
            Node::rect()
                .color(Color::WHITE)
                .height(30.0 * k)
                .anchors(Anchors::default().left(0.0).right(0.0).top(0.0))
                .layout(Layout::row(6.0 * k).padding(4.0 * k, 3.0 * k)),
        );
        for w in [20.0, 35.0, 12.0] {
            s.add(bar, Node::rect().color(Color::BLACK).size(w * k, 18.0 * k).radius(4.0 * k));
        }
        s.add(root, Node::rect().color(Color::BLACK).size(10.0 * k, 10.0 * k).anchors(Anchors::center()));
    }

    #[test]
    fn logical_scene_matches_premultiplied_pixels_at_integer_scale() {
        for (ui, output) in [(2.0, 1.0), (2.0, 2.0), (3.0, 1.0)] {
            let mut logical = Scene::new(Fonts::new());
            logical.set_viewport(Vec2::new(600.0, 300.0), output);
            logical.set_ui_scale(ui);
            build(&mut logical, 1.0);
            let mut pixels = Scene::new(Fonts::new());
            pixels.set_viewport(Vec2::new(600.0, 300.0), output);
            build(&mut pixels, ui);
            assert_eq!(quads(&mut logical), quads(&mut pixels), "ui {ui} output {output}");
        }
    }

    #[test]
    fn fractional_scale_snaps_edges_without_accumulated_drift() {
        let mut s = Scene::new(Fonts::new());
        s.set_viewport(Vec2::new(500.0, 100.0), 1.0);
        s.set_ui_scale(1.25);
        let root = s.root();
        let row = s.add(root, Node::item().layout(Layout::row(6.0)).x(10.0).y(10.0));
        for _ in 0..10 {
            s.add(row, Node::rect().color(Color::WHITE).size(10.0, 10.0).border(1.0, Color::BLACK));
        }
        let rects = quads(&mut s);
        assert_eq!(rects.len(), 10);
        for (i, r) in rects.iter().enumerate() {
            let left = 10.0 + i as f32 * 16.0;
            assert_eq!((r.x, r.right()), (scaled(left, 1.25), scaled(left + 10.0, 1.25)), "{i}");
            assert!(r.w == 12.0 || r.w == 13.0);
        }
        let mut per_literal = scaled(10.0, 1.25);
        for _ in 0..9 {
            per_literal += scaled(10.0, 1.25) + scaled(6.0, 1.25);
        }
        assert_eq!(rects[9].right(), 205.0);
        assert_eq!(per_literal + scaled(10.0, 1.25), 215.0);
        let border = s.frame().primitives.iter().find_map(|p| match p {
            Primitive::Quad(q) => Some(q.border_width),
            _ => None,
        });
        assert_eq!(border, Some(1.0));
    }

    #[test]
    fn viewport_and_bounds_are_logical() {
        let mut s = Scene::new(Fonts::new());
        s.set_viewport(Vec2::new(1000.0, 50.0), 1.0);
        s.set_ui_scale(1.25);
        assert_eq!(s.viewport().get_untracked(), Vec2::new(800.0, 40.0));
        let root = s.root();
        let fill = s.add(root, Node::rect().anchors(Anchors::fill(0.0)));
        s.frame();
        assert_eq!(s.geometry(fill), Some(Rect::new(0.0, 0.0, 800.0, 40.0)));
        assert_eq!(s.scale(), 1.25);
        s.set_viewport(Vec2::new(1000.0, 50.0), 2.0);
        assert_eq!(s.scale(), 2.5);
        assert_eq!(s.viewport().get_untracked(), Vec2::new(800.0, 40.0));
    }

    #[test]
    fn pointer_and_scroll_arrive_in_logical_units() {
        let mut s = Scene::new(Fonts::new());
        s.set_viewport(Vec2::new(400.0, 200.0), 1.0);
        s.set_ui_scale(2.0);
        let root = s.root();
        let clicks = Signal::new(0);
        let scrolled = Signal::new(ScrollDelta::default());
        let (c, d) = (clicks.clone(), scrolled.clone());
        let area = MouseArea::new().on_click(move |_| c.update(|n| *n += 1)).on_scroll(move |delta| d.set(delta));
        let target = s.add(root, Node::rect().x(100.0).y(50.0).size(20.0, 20.0).mouse(area));
        s.frame();
        s.pointer_motion(Vec2::new(205.0, 105.0));
        assert!(s.get(target).unwrap().mouse.as_ref().unwrap().hovered.get_untracked());
        s.pointer_button(MouseButton::Left, true);
        s.pointer_button(MouseButton::Left, false);
        assert_eq!(clicks.get_untracked(), 1);
        s.scroll(ScrollDelta { x: 0.0, y: 30.0 });
        assert_eq!(scrolled.get_untracked(), ScrollDelta { x: 0.0, y: 15.0 });
        s.pointer_motion(Vec2::new(190.0, 105.0));
        assert!(!s.get(target).unwrap().mouse.as_ref().unwrap().hovered.get_untracked());
    }

    #[test]
    fn a_bound_scale_relays_out_and_converts_surface_geometry() {
        let mut s = Scene::new(Fonts::new());
        s.set_viewport(Vec2::new(1000.0, 30.0), 1.0);
        let ui = Signal::new(1.0f32);
        s.set_ui_scale(ui.clone());
        s.input_region.set(Some(vec![Rect::new(10.0, 0.0, 30.0, 30.0)]));
        s.layer_geometry.set(Some(LayerGeometry {
            top: true,
            bottom: false,
            left: true,
            right: true,
            size: (0, 30),
            margin: [5, 0, 0, 0],
            exclusive_zone: 30,
        }));
        let root = s.root();
        let fill = s.add(root, Node::rect().anchors(Anchors::fill(0.0)));
        s.frame();
        assert!(!s.needs_frame());
        ui.set(1.25);
        assert!(s.needs_frame());
        s.frame();
        assert_eq!(s.ui_scale(), 1.25);
        assert_eq!(s.geometry(fill), Some(Rect::new(0.0, 0.0, 800.0, 24.0)));
        assert_eq!(s.surface_input_region(), Some(vec![Rect::new(12.5, 0.0, 37.5, 37.5)]));
        let g = s.surface_layer_geometry().unwrap();
        assert_eq!((g.size, g.margin, g.exclusive_zone), ((0, 38), [6, 0, 0, 0], 38));
        ui.set(-1.0);
        s.frame();
        assert_eq!(s.ui_scale(), 1.0);
    }
}
