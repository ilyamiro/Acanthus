//! Value slider from 0 to 100, horizontal or vertical: a gradient fill up to the handle, a handle
//! that grows while dragged, wheel steps and arrow, Home and End keys.

use crate::component::{Component, Kind, Prop, PropSpec, Props};
use crate::timing::{BASE, FAST};
use crate::wheel::{WHEEL_UNITS_PER_ABSOLUTE, Wheel};
use std::cell::RefCell;
use std::rc::Rc;

use acanthus_anim::Easing;
use acanthus_core::{Color, Signal, lighten};
use acanthus_scene::{Anchors, CursorShape, Gradient, Key, MouseArea, Node, NodeId, PointerPhase, Scene};

use acanthus_anim::{Animated, Keep};

/// Value under a pointer at `position` along a track of length `extent`, mapped to `from..=to`;
/// vertical tracks grow upwards.
pub fn value_at(position: f32, extent: f32, vertical: bool, from: f32, to: f32) -> f32 {
    let extent = extent.max(1.0);
    let fraction = if vertical { (extent - position) / extent } else { position / extent };
    (from + fraction.clamp(0.0, 1.0) * (to - from)).clamp(from, to)
}

/// Fraction of the track (0 to 1) that `value` fills within `from..=to`.
pub fn visual_position(value: f32, from: f32, to: f32) -> f32 {
    ((value - from) / (to - from).max(0.0001)).clamp(0.0, 1.0)
}

const KEY_STEP: f32 = (TO - FROM) / 20.0;

/// Value after pressing `key` at `current`: arrows move by a twentieth of the range, Home and End
/// jump to the ends; `None` for other keys.
pub fn key_value(key: Key, current: f32) -> Option<f32> {
    let next = match key {
        Key::Right | Key::Up => current + KEY_STEP,
        Key::Left | Key::Down => current - KEY_STEP,
        Key::Home => FROM,
        Key::End => TO,
        _ => return None,
    };
    Some(next.clamp(FROM, TO))
}

/// Description of a slider.
pub struct SliderSpec {
    /// Lays the track out vertically, growing upwards.
    pub vertical: bool,
    /// Horizontal position inside the parent.
    pub x: Signal<f32>,
    /// Vertical position inside the parent.
    pub y: Signal<f32>,
    /// Width of the track.
    pub width: Signal<f32>,
    /// Height of the track.
    pub height: Signal<f32>,
    /// Opacity of the whole slider.
    pub opacity: Signal<f32>,
    /// Whether the slider is shown.
    pub visible: Signal<bool>,
    /// Committed value from 0 to 100; the slider shows it whenever the user is not dragging.
    pub value: Signal<f32>,
    /// Corner radius of the track.
    pub corner_radius: f32,
    /// Diameter of the handle at rest.
    pub handle_size: f32,
    /// Colour of the empty track.
    pub background: Signal<Color>,
    /// Colour of the filled part.
    pub accent: Signal<Color>,
    /// Handle colour at rest.
    pub handle: Signal<Color>,
    /// Handle colour while dragged.
    pub handle_drag: Signal<Color>,
    /// Called when a drag begins.
    pub on_drag_start: Rc<dyn Fn()>,
    /// Called with the new value on every drag move, wheel step or key press.
    pub on_moved: Rc<dyn Fn(f32)>,
    /// Called when a drag ends.
    pub on_drag_finish: Rc<dyn Fn()>,
}

/// A built slider.
pub struct Slider {
    /// Outer node of the slider.
    pub node: NodeId,
    /// Value currently shown: the drag preview while dragging, [`SliderSpec::value`] otherwise.
    pub effective: Signal<f32>,
}

const FROM: f32 = 0.0;
const TO: f32 = 100.0;

/// Builds a slider under `parent`.
pub fn build(scene: &mut Scene, parent: NodeId, spec: SliderSpec) -> Slider {
    let mut keep = Keep::default();
    let dragging = Signal::new(false);
    let preview = Signal::new(0.0f32);
    let effective = {
        let (dragging, preview, value) = (dragging.clone(), preview.clone(), spec.value.clone());
        keep.derive(move || if dragging.get() { preview.get() } else { value.get() })
    };
    let target = {
        let effective = effective.clone();
        keep.derive(move || visual_position(effective.get(), FROM, TO))
    };
    let position = {
        let dragging = dragging.clone();
        let eased =
            Animated::with(&target, move |_| (if dragging.get_untracked() { 0 } else { BASE }, Easing::OutCubic));
        let signal = eased.signal().clone();
        keep.add(eased);
        signal
    };
    let handle_scale = {
        let dragging = dragging.clone();
        let wanted = keep.derive(move || if dragging.get() { 1.4f32 } else { 0.0 });
        keep.ease(&wanted, FAST, Easing::OUT_BACK)
    };
    let fill_opacity = {
        let position = position.clone();
        let wanted = keep.derive(move || if position.get() > 0.001 { 1.0f32 } else { 0.0 });
        keep.ease(&wanted, FAST, Easing::OutCubic)
    };
    let (vertical, size) = (spec.vertical, spec.handle_size);
    let handle_x = {
        let (position, width) = (position.clone(), spec.width.clone());
        keep.derive(move || if vertical { (width.get() - size) / 2.0 } else { position.get() * (width.get() - size) })
    };
    let handle_y =
        {
            let (position, height) = (position.clone(), spec.height.clone());
            keep.derive(move || {
                if vertical { (1.0 - position.get()) * (height.get() - size) } else { (height.get() - size) / 2.0 }
            })
        };
    let fill_x = keep.derive(|| 0.0f32);
    let fill_y = {
        let (handle_y, height) = (handle_y.clone(), spec.height.clone());
        keep.derive(move || if vertical { handle_y.get().clamp(0.0, height.get()) } else { 0.0 })
    };
    let fill_width = {
        let (handle_x, width) = (handle_x.clone(), spec.width.clone());
        keep.derive(move || if vertical { width.get() } else { (handle_x.get() + size).clamp(0.0, width.get()) })
    };
    let fill_height = {
        let (fill_y, height) = (fill_y.clone(), spec.height.clone());
        keep.derive(move || if vertical { (height.get() - fill_y.get()).max(0.0) } else { height.get() })
    };
    let gradient = {
        let accent = spec.accent.clone();
        keep.derive(move || {
            let (first, last) = (accent.get(), lighten(accent.get(), 1.10));
            Some(if vertical { Gradient::vertical(last, first) } else { Gradient::horizontal(first, last) })
        })
    };
    let handle_color = {
        let (dragging, normal, drag) = (dragging.clone(), spec.handle.clone(), spec.handle_drag.clone());
        keep.derive(move || if dragging.get() { drag.get() } else { normal.get() })
    };

    let extent = {
        let (width, height) = (spec.width.clone(), spec.height.clone());
        move || if vertical { height.get_untracked() } else { width.get_untracked() }
    };
    let update = Rc::new({
        let (preview, on_moved, extent) = (preview.clone(), spec.on_moved.clone(), extent.clone());
        move |position: f32| {
            let next = value_at(position, extent(), vertical, FROM, TO);
            preview.set(next);
            on_moved(next);
        }
    });
    let pointer = {
        let (dragging, preview, value, update) = (dragging.clone(), preview.clone(), spec.value.clone(), update);
        let (start, finish) = (spec.on_drag_start.clone(), spec.on_drag_finish.clone());
        move |event: acanthus_scene::PointerEvent| {
            let along = if vertical { event.position.y } else { event.position.x };
            match event.phase {
                PointerPhase::Press => {
                    dragging.set(true);
                    preview.set(value.get_untracked());
                    start();
                    update(along);
                }
                PointerPhase::Drag => {
                    if dragging.get_untracked() {
                        update(along);
                    }
                }
                PointerPhase::Release => {
                    dragging.set(false);
                    finish();
                }
            }
        }
    };
    let wheel = {
        let (effective, preview, on_moved) = (effective.clone(), preview.clone(), spec.on_moved.clone());
        let state = RefCell::new(Wheel::default());
        MouseArea::new().on_scroll(move |delta| {
            let steps = state.borrow_mut().feed(-delta.y * WHEEL_UNITS_PER_ABSOLUTE);
            if steps == 0 {
                return;
            }
            let next = (effective.get_untracked() + steps as f32).clamp(FROM, TO);
            if next != effective.get_untracked() {
                preview.set(next);
                on_moved(next);
            }
        })
    };

    let key_handler = {
        let (effective, preview, on_moved) = (effective.clone(), preview.clone(), spec.on_moved.clone());
        move |event: &acanthus_scene::KeyEvent| {
            let Some(next) = key_value(event.key, effective.get_untracked()) else {
                return false;
            };
            if next != effective.get_untracked() {
                preview.set(next);
                on_moved(next);
            }
            true
        }
    };
    let radius = spec.corner_radius;
    let outer = scene.add(
        parent,
        Node::item()
            .x(&spec.x)
            .y(&spec.y)
            .width(&spec.width)
            .height(&spec.height)
            .opacity(&spec.opacity)
            .visible(&spec.visible)
            .mouse(wheel)
            .cursor(CursorShape::Pointer)
            .on_pointer(pointer)
            .focusable(true)
            .on_key(key_handler)
            .own(keep),
    );
    let track =
        scene.add(outer, Node::rect().anchors(Anchors::fill(0.0)).radius(radius).color(&spec.background).clip(true));
    scene.add(
        track,
        Node::rect()
            .x(&fill_x)
            .y(&fill_y)
            .width(&fill_width)
            .height(&fill_height)
            .radius(radius)
            .color(Color::TRANSPARENT)
            .gradient(&gradient)
            .opacity(&fill_opacity),
    );
    scene.add(
        outer,
        Node::rect()
            .x(&handle_x)
            .y(&handle_y)
            .size(size, size)
            .radius(size / 2.0)
            .color(&handle_color)
            .scale(&handle_scale),
    );
    Slider { node: outer, effective }
}

/// The slider as a [`Component`] named `Slider`.
pub struct SliderComponent;

impl Component for SliderComponent {
    fn name(&self) -> &'static str {
        "Slider"
    }

    fn props(&self) -> &'static [PropSpec] {
        const PROPS: &[PropSpec] = &[
            PropSpec::bool("vertical", || Prop::bool(false)),
            PropSpec::number("x", || Prop::number(0.0)),
            PropSpec::number("y", || Prop::number(0.0)),
            PropSpec::number("width", || Prop::number(120.0)),
            PropSpec::number("height", || Prop::number(16.0)),
            PropSpec::number("opacity", || Prop::number(1.0)),
            PropSpec::bool("visible", || Prop::bool(true)),
            PropSpec::number("value", || Prop::number(0.0)),
            PropSpec::number("corner_radius", || Prop::number(8.0)),
            PropSpec::number("handle_size", || Prop::number(16.0)),
            PropSpec::required("background", Kind::Color),
            PropSpec::required("accent", Kind::Color),
            PropSpec::required("handle", Kind::Color),
            PropSpec::required("handle_drag", Kind::Color),
            PropSpec::event("on_drag_start"),
            PropSpec::number_event("on_moved"),
            PropSpec::event("on_drag_finish"),
        ];
        PROPS
    }

    fn build(&self, scene: &mut Scene, parent: NodeId, props: &Props) -> NodeId {
        let spec = SliderSpec {
            vertical: props.bool("vertical").get_untracked(),
            x: props.number("x"),
            y: props.number("y"),
            width: props.number("width"),
            height: props.number("height"),
            opacity: props.number("opacity"),
            visible: props.bool("visible"),
            value: props.number("value"),
            corner_radius: props.number("corner_radius").get_untracked(),
            handle_size: props.number("handle_size").get_untracked(),
            background: props.color("background"),
            accent: props.color("accent"),
            handle: props.color("handle"),
            handle_drag: props.color("handle_drag"),
            on_drag_start: props.event("on_drag_start"),
            on_moved: props.number_event("on_moved"),
            on_drag_finish: props.event("on_drag_finish"),
        };
        build(scene, parent, spec).node
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn value_maths() {
        assert_eq!(value_at(50.0, 200.0, false, 0.0, 100.0), 25.0);
        assert_eq!(value_at(-30.0, 200.0, false, 0.0, 100.0), 0.0);
        assert_eq!(value_at(500.0, 200.0, false, 0.0, 100.0), 100.0);
        assert_eq!(value_at(0.0, 200.0, true, 0.0, 100.0), 100.0);
        assert_eq!(value_at(150.0, 200.0, true, 0.0, 100.0), 25.0);
        assert_eq!(visual_position(25.0, 0.0, 100.0), 0.25);
        assert_eq!(visual_position(250.0, 0.0, 100.0), 1.0);
    }

    #[test]
    fn keys_step_by_five_and_clamp() {
        assert_eq!(key_value(Key::Right, 10.0), Some(15.0));
        assert_eq!(key_value(Key::Up, 98.0), Some(100.0));
        assert_eq!(key_value(Key::Left, 3.0), Some(0.0));
        assert_eq!(key_value(Key::Down, 50.0), Some(45.0));
        assert_eq!(key_value(Key::Home, 50.0), Some(0.0));
        assert_eq!(key_value(Key::End, 50.0), Some(100.0));
        assert_eq!(key_value(Key::Tab, 50.0), None);
    }

    #[test]
    fn the_registry_builds_a_slider_and_rejects_a_misspelt_prop() {
        use crate::component::{BuildError, Registry};
        let mut registry = Registry::new();
        registry.register(SliderComponent);
        let colors = || {
            ["background", "accent", "handle", "handle_drag"]
                .map(|name| (name.to_owned(), Prop::color(Color::hex(0x336699))))
                .to_vec()
        };
        let mut scene = Scene::new(acanthus_scene::Fonts::new());
        let root = scene.root();
        let mut given = colors();
        given.push(("value".to_owned(), Prop::number(40.0)));
        let node = registry.build("Slider", &mut scene, root, given).unwrap();
        assert!(scene.get(node).is_some());
        let mut typo = colors();
        typo.push(("handle_sise".to_owned(), Prop::number(10.0)));
        let err = registry.build("Slider", &mut scene, root, typo).unwrap_err();
        assert!(matches!(err, BuildError::UnknownProp { suggestion: Some(ref s), .. } if s == "handle_size"));
        let err = registry.build("Slider", &mut scene, root, vec![]).unwrap_err();
        assert_eq!(err, BuildError::Missing { component: "Slider", name: "background" });
    }
}
