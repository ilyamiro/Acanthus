//! Thin scroll indicator for a [`Flickable`]: visible only while the content moves and only when
//! the content is taller than the view.

use crate::component::{Component, Kind, Prop, PropSpec, Props};
use acanthus_anim::Keep;
use acanthus_core::{Color, Signal, Vec2};
use acanthus_scene::{Flickable, IntoSignal, Node, NodeId, Scene};

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
    /// Flickable whose content height, position and motion the bar follows.
    pub flickable: Flickable,
    /// Size of the viewport.
    pub view: Signal<Vec2>,
    /// Horizontal position of the bar inside its parent.
    pub x: X,
    /// Vertical offset of the track inside its parent.
    pub top: f32,
    /// Width of the bar.
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
    let ScrollBar { flickable, view, x, top, width, radius, color } = bar;
    let handle = {
        let flick = flickable.clone();
        keep.derive(move || scroll_handle(flick.content_height.get(), view.get().y, flick.content_y.get()))
    };
    let visible = {
        let (handle, moving) = (handle.clone(), flickable.moving.clone());
        keep.derive(move || handle.get().is_some() && moving.get())
    };
    let y = {
        let handle = handle.clone();
        keep.derive(move || handle.get().map_or(0.0, |h| h.0 + top))
    };
    let height = keep.derive(move || handle.get().map_or(0.0, |h| h.1));
    scene.add(
        parent,
        Node::rect().x(x).y(&y).size(width, &height).radius(radius).color(color).visible(&visible).own(keep),
    )
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
            PropSpec::required("color", Kind::Color),
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
