//! Scrolling label: text wider than its box scrolls left in a loop, with a pause at the start of
//! every cycle and a second copy following at a fixed spacing so the text wraps around seamlessly.

use std::cell::RefCell;
use std::rc::Rc;

use crate::component::{Component, Prop, PropSpec, Props};
use acanthus_anim::{Action, AnimHandle, Easing, Keep, Loop, Pause, Sequential, Tween, start};
use acanthus_core::{Color, Signal};
use acanthus_scene::{Layout, Node, NodeId, Scene};

/// Pause before each scroll cycle, in milliseconds.
pub const MARQUEE_PAUSE_MS: u32 = 3000;

/// Whether a label scrolls: it must be active and wider than a non-empty box.
pub fn can_marquee(active: bool, text_width: f32, width: f32) -> bool {
    active && text_width > width && width > 0.0
}

/// Distance of one cycle: the text width plus the spacing to the following copy.
pub fn scroll_distance(text_width: f32, spacing: f32) -> f32 {
    text_width + spacing
}

/// Duration of one cycle over `distance` pixels at `speed` milliseconds per pixel, at least one second.
pub fn scroll_duration_ms(distance: f32, speed: f32) -> u32 {
    (distance * speed).round().max(1000.0) as u32
}

/// Horizontal offset of the text at `progress` (0 to 1) through a cycle, rounded to whole pixels.
pub fn scroll_x(progress: f32, distance: f32) -> f32 {
    -(progress * distance).round()
}

/// Description of a marquee label.
pub struct Spec {
    /// Text shown.
    pub text: Signal<String>,
    /// Text colour.
    pub color: Signal<Color>,
    /// Font family.
    pub family: Signal<String>,
    /// Font size.
    pub size: f32,
    /// Font weight (400 regular, 700 bold).
    pub weight: u16,
    /// Width of the visible box; the text is clipped to it.
    pub width: Signal<f32>,
    /// Gap between the text and its following copy.
    pub spacing: f32,
    /// Scroll speed in milliseconds per pixel (larger is slower).
    pub speed: f32,
    /// Whether scrolling is allowed; inactive labels stay still and clipped.
    pub active: Signal<bool>,
}

struct Run(RefCell<Option<AnimHandle>>);

impl Drop for Run {
    fn drop(&mut self) {
        if let Some(handle) = self.0.borrow_mut().take() {
            handle.stop();
        }
    }
}

/// Builds the label under `parent` and returns its clipping container. The cycle restarts when
/// the text, its width or the box width changes.
pub fn build(scene: &mut Scene, parent: NodeId, spec: Spec) -> NodeId {
    let Spec { text, color, family, size, weight, width, spacing, speed, active } = spec;
    let mut keep = Keep::default();
    let label = |visible: Option<&Signal<bool>>| {
        let node = Node::text(&text).font_family(&family).font_size(size).font_weight(weight).color(&color);
        match visible {
            Some(visible) => node.visible(visible),
            None => node,
        }
    };
    let main = label(None);
    let measured = main.implicit_size();
    let height = {
        let measured = measured.clone();
        keep.derive(move || measured.get().y)
    };
    let can = {
        let (measured, active, width) = (measured.clone(), active, width.clone());
        keep.derive(move || can_marquee(active.get(), measured.get().x, width.get()))
    };
    let distance = {
        let measured = measured.clone();
        keep.derive(move || scroll_distance(measured.get().x, spacing))
    };
    let progress = Signal::new(0.0f32);
    let run = Rc::new(Run(RefCell::default()));
    {
        let (can, distance, text, progress, run) =
            (can.clone(), distance.clone(), text.clone(), progress.clone(), run.clone());
        keep.add(Signal::new(()).bind(move || {
            let (running, distance) = (can.get(), distance.get());
            text.get();
            if let Some(old) = run.0.borrow_mut().take() {
                old.stop();
            }
            progress.set(0.0);
            if running {
                let reset = {
                    let progress = progress.clone();
                    Action::new(move || progress.set(0.0))
                };
                let cycle = Sequential::new()
                    .then(Pause::new(MARQUEE_PAUSE_MS))
                    .then(Tween::new(&progress, 1.0, scroll_duration_ms(distance, speed), Easing::Linear))
                    .then(reset);
                *run.0.borrow_mut() = Some(start(Loop::infinite(cycle)));
            }
        }));
    }
    keep.add(run);
    let x = {
        let (can, progress, distance) = (can.clone(), progress, distance);
        keep.derive(move || if can.get() { scroll_x(progress.get(), distance.get()) } else { 0.0 })
    };

    let container = scene.add(parent, Node::item().width(&width).height(&height).clip(true).own(keep));
    let row = scene.add(container, Node::item().layout(Layout::row(spacing)).x(&x));
    scene.add(row, main);
    scene.add(row, label(Some(&can)));
    container
}

/// The marquee label as a [`Component`] named `Marquee`.
pub struct MarqueeComponent;

impl Component for MarqueeComponent {
    fn name(&self) -> &'static str {
        "Marquee"
    }

    fn props(&self) -> &'static [PropSpec] {
        const PROPS: &[PropSpec] = &[
            PropSpec::text("text", || Prop::text("")),
            PropSpec::text("family", || Prop::text("")),
            PropSpec::number("size", || Prop::number(14.0)),
            PropSpec::number("weight", || Prop::number(400.0)),
            PropSpec::number("width", || Prop::number(120.0)),
            PropSpec::number("spacing", || Prop::number(40.0)),
            PropSpec::number("speed", || Prop::number(30.0)),
            PropSpec::bool("active", || Prop::bool(true)),
            PropSpec::themed("color", "text"),
        ];
        PROPS
    }

    fn build(&self, scene: &mut Scene, parent: NodeId, props: &Props) -> NodeId {
        build(
            scene,
            parent,
            Spec {
                text: props.text("text"),
                color: props.color("color"),
                family: props.text("family"),
                size: props.number("size").get_untracked(),
                weight: props.number("weight").get_untracked().clamp(1.0, 1000.0) as u16,
                width: props.number("width"),
                spacing: props.number("spacing").get_untracked(),
                speed: props.number("speed").get_untracked(),
                active: props.bool("active"),
            },
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn marquee_maths() {
        assert!(can_marquee(true, 200.0, 120.0));
        assert!(!can_marquee(false, 200.0, 120.0));
        assert!(!can_marquee(true, 100.0, 120.0));
        assert!(!can_marquee(true, 100.0, 0.0));
        assert_eq!(scroll_distance(200.0, 40.0), 240.0);
        assert_eq!(scroll_duration_ms(240.0, 30.0), 7200);
        assert_eq!(scroll_duration_ms(10.0, 30.0), 1000);
        assert_eq!(scroll_x(0.5, 241.0), -121.0);
    }

    #[test]
    fn the_registry_builds_a_marquee_with_a_numeric_weight() {
        let mut registry = crate::component::Registry::new();
        registry.register(MarqueeComponent);
        let mut scene = Scene::new(acanthus_scene::Fonts::new());
        let root = scene.root();
        let given =
            vec![("color".to_owned(), Prop::color(Color::hex(0xffffff))), ("weight".to_owned(), Prop::number(700.0))];
        let node = registry.build("Marquee", &mut scene, root, given).unwrap();
        assert!(scene.get(node).is_some());
        let err = registry.build("Marquee", &mut scene, root, vec![]).unwrap_err();
        assert_eq!(err.to_string(), "`Marquee` needs the prop `color`");
    }
}
