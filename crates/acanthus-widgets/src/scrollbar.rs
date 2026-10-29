//! Thin scroll indicator for a [`Flickable`]: visible only while the content moves and only when
//! the content is longer than the view; and [`scroll_view`], a flickable with one on each axis.

use crate::component::{Component, Kind, Prop, PropSpec, Props};
use acanthus_anim::Keep;
use acanthus_core::{Color, Signal, Vec2};
use acanthus_scene::{Anchors, Flickable, FlickableNodes, IntoSignal, Node, NodeId, Orientation, Scene};

/// Position and length of the handle for `content` tall content in a `view` tall viewport
/// scrolled to `position`; `None` when everything fits.
pub fn scroll_handle(content: f32, view: f32, position: f32) -> Option<(f32, f32)> {
    if content <= view || view <= 0.0 {
        return None;
    }
    let height = view * view / content;
    let travel = view - height;
    let max = content - view;
    Some((position.clamp(0.0, max) / max * travel, height))
}

/// Description of a scroll bar.
pub struct ScrollBar<X, C> {
    /// Flickable whose content length, position and motion the bar follows.
    pub flickable: Flickable,
    /// Size of the viewport.
    pub view: Signal<Vec2>,
    /// Axis the bar scrolls: a vertical bar follows the content height, a horizontal one the content width.
    pub orientation: Orientation,
    /// Position of the bar across its axis inside its parent: horizontal for a vertical bar, vertical for a horizontal one.
    pub x: X,
    /// Offset of the track along its axis inside its parent.
    pub top: f32,
    /// Thickness of the bar.
    pub width: f32,
    /// Corner radius of the handle.
    pub radius: f32,
    /// Colour of the handle.
    pub color: C,
}

/// Builds the scroll bar handle under `parent` and returns it.
pub fn scroll_bar<X: IntoSignal<f32>, C: IntoSignal<Color>>(
    scene: &mut Scene,
    parent: NodeId,
    bar: ScrollBar<X, C>,
) -> NodeId {
    let mut keep = Keep::default();
    let ScrollBar { flickable, view, orientation, x, top, width, radius, color } = bar;
    let vertical = orientation == Orientation::Vertical;
    let handle = {
        let flick = flickable.clone();
        keep.derive(move || {
            if vertical {
                scroll_handle(flick.content_height.get(), view.get().y, flick.content_y.get())
            } else {
                scroll_handle(flick.content_width.get(), view.get().x, flick.content_x.get())
            }
        })
    };
    let visible = {
        let (handle, moving) = (handle.clone(), flickable.moving.clone());
        keep.derive(move || handle.get().is_some() && moving.get())
    };
    let y = {
        let handle = handle.clone();
        keep.derive(move || handle.get().map_or(0.0, |h| h.0 + top))
    };
    let length = keep.derive(move || handle.get().map_or(0.0, |h| h.1));
    let node = if vertical {
        Node::rect().x(x).y(&y).size(width, &length)
    } else {
        Node::rect().y(x).x(&y).size(&length, width)
    };
    scene.add(parent, node.radius(radius).color(color).visible(&visible).own(keep))
}

/// Description of a scroll view.
pub struct ScrollView<C> {
    /// The frame holding the view: give it a position and a size, or anchors.
    pub frame: Node,
    /// Thickness of each bar.
    pub bar_width: f32,
    /// Gap between a bar and the frame's edge.
    pub bar_margin: f32,
    /// Corner radius of the handles.
    pub radius: f32,
    /// Colour of the handles.
    pub color: C,
}

/// Builds a QML `ScrollView` under `parent`: a clipped [`Flickable`] filling `frame` whose content size follows what is added under the returned content node, with a vertical bar along the right edge and a horizontal bar along the bottom, each shown as [`scroll_bar`] shows it. Returns the nodes and the flickable.
pub fn scroll_view<C: IntoSignal<Color> + Clone>(
    scene: &mut Scene,
    parent: NodeId,
    spec: ScrollView<C>,
) -> (FlickableNodes, Flickable) {
    let ScrollView { frame, bar_width, bar_margin, radius, color } = spec;
    let frame = scene.add(parent, frame);
    let flickable = Flickable::new();
    let (nodes, view) = scene.scroll_view(frame, Node::item().anchors(Anchors::fill(0.0)), &flickable);
    let edge = bar_width + bar_margin;
    for orientation in [Orientation::Vertical, Orientation::Horizontal] {
        let mut keep = Keep::default();
        let x = {
            let view = view.clone();
            let vertical = orientation == Orientation::Vertical;
            keep.derive(move || if vertical { view.get().x - edge } else { view.get().y - edge })
        };
        let bar = ScrollBar {
            flickable: flickable.clone(),
            view: view.clone(),
            orientation,
            x: &x,
            top: 0.0,
            width: bar_width,
            radius,
            color: color.clone(),
        };
        let id = scroll_bar(scene, frame, bar);
        if let Some(node) = scene.get_mut(id) {
            node.keep(keep);
        }
    }
    (nodes, flickable)
}

/// The scroll bar as a [`Component`] named `ScrollBar`.
pub struct ScrollBarComponent;

impl Component for ScrollBarComponent {
    fn name(&self) -> &'static str {
        "ScrollBar"
    }

    fn props(&self) -> &'static [PropSpec] {
        const PROPS: &[PropSpec] = &[
            PropSpec::number("x", || Prop::number(0.0)),
            PropSpec::number("top", || Prop::number(0.0)),
            PropSpec::number("width", || Prop::number(4.0)),
            PropSpec::number("radius", || Prop::number(2.0)),
            PropSpec::point("view", || Prop::Point(Signal::new(Vec2::ZERO))),
            PropSpec::required("flickable", Kind::Flickable),
            PropSpec::themed("color", "overlay1"),
        ];
        PROPS
    }

    fn build(&self, scene: &mut Scene, parent: NodeId, props: &Props) -> NodeId {
        scroll_bar(
            scene,
            parent,
            ScrollBar {
                flickable: props.flickable("flickable"),
                view: props.point("view"),
                orientation: Orientation::Vertical,
                x: props.number("x"),
                top: props.number("top").get_untracked(),
                width: props.number("width").get_untracked(),
                radius: props.number("radius").get_untracked(),
                color: props.color("color"),
            },
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn handle_is_hidden_when_the_content_fits() {
        assert_eq!(scroll_handle(300.0, 300.0, 0.0), None);
        assert_eq!(scroll_handle(200.0, 300.0, 0.0), None);
        assert_eq!(scroll_handle(500.0, 0.0, 0.0), None);
    }

    #[test]
    fn handle_follows_the_position_over_the_travel() {
        assert_eq!(scroll_handle(600.0, 300.0, 0.0), Some((0.0, 150.0)));
        assert_eq!(scroll_handle(600.0, 300.0, 150.0), Some((75.0, 150.0)));
        assert_eq!(scroll_handle(600.0, 300.0, 300.0), Some((150.0, 150.0)));
        assert_eq!(scroll_handle(600.0, 300.0, 900.0), Some((150.0, 150.0)));
        assert_eq!(scroll_handle(600.0, 300.0, -20.0), Some((0.0, 150.0)));
    }

    #[test]
    fn scroll_view_shows_a_bar_per_overflowing_axis_while_moving() {
        let mut scene = Scene::new(acanthus_scene::Fonts::new());
        scene.set_viewport(Vec2::new(200.0, 200.0), 1.0);
        let root = scene.root();
        let spec = ScrollView {
            frame: Node::item().x(10.0).size(100.0, 50.0),
            bar_width: 4.0,
            bar_margin: 2.0,
            radius: 2.0,
            color: Color::hex(0x888888),
        };
        let (nodes, flickable) = scroll_view(&mut scene, root, spec);
        let content = scene.add(nodes.content, Node::rect().size(80.0, 200.0));
        scene.frame();
        assert_eq!(flickable.content_height.get_untracked(), 200.0);
        let frame = scene.get(nodes.viewport).unwrap().parent().unwrap();
        let bars: Vec<NodeId> = scene.get(frame).unwrap().children()[1..].to_vec();
        let shown =
            |scene: &Scene| bars.iter().map(|b| scene.get(*b).unwrap().visible.get_untracked()).collect::<Vec<_>>();
        assert_eq!(shown(&scene), vec![false, false]);
        flickable.moving.set(true);
        flickable.content_y.set(75.0);
        scene.frame();
        assert_eq!(shown(&scene), vec![true, false]);
        assert_eq!(scene.geometry(bars[0]), Some(acanthus_core::Rect::new(104.0, 18.75, 4.0, 12.5)));
        scene.get(content).unwrap().width.as_ref().unwrap().set(400.0);
        scene.frame();
        assert_eq!(shown(&scene), vec![true, true]);
        assert_eq!(scene.geometry(bars[1]).map(|r| (r.y, r.w, r.h)), Some((44.0, 25.0, 4.0)));
    }

    #[test]
    fn the_registry_builds_a_scroll_bar_that_follows_its_flickable() {
        let mut registry = crate::component::Registry::new();
        registry.register(ScrollBarComponent);
        let flickable = Flickable::default();
        flickable.content_height.set(600.0);
        let view = Signal::new(Vec2::new(100.0, 300.0));
        let given = vec![
            ("flickable".to_owned(), Prop::Flickable(flickable.clone())),
            ("view".to_owned(), Prop::Point(view)),
            ("color".to_owned(), Prop::color(Color::hex(0x888888))),
        ];
        let mut scene = Scene::new(acanthus_scene::Fonts::new());
        let root = scene.root();
        let node = registry.build("ScrollBar", &mut scene, root, given).unwrap();
        assert!(scene.get(node).is_some());
    }
}
