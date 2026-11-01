//! Progress bar (determinate or sliding indeterminate segment) and spinner (a rotating arc).

use std::f32::consts::TAU;

use acanthus_anim::{Easing, Keep};
use acanthus_core::{Color, Signal, Vec2};
use acanthus_scene::{Node, NodeId, Path, Scene};

use crate::component::{Component, Prop, PropSpec, Props};
use crate::timing::FAST;

/// Duration of one sweep of the indeterminate segment, in milliseconds.
pub const INDETERMINATE_MS: u32 = 1400;

/// Duration of one turn of the spinner, in milliseconds.
pub const SPINNER_TURN_MS: u32 = 1000;

/// Share of the bar the indeterminate segment covers.
pub const SEGMENT_SHARE: f32 = 0.4;

/// Width of the filled part for `value` (clamped to 0 to 1) on a bar of `width`.
pub fn fill_width(value: f32, width: f32) -> f32 {
    value.clamp(0.0, 1.0) * width
}

/// Left edge of the indeterminate segment at `phase` (0 to 1): it enters from the left and leaves at the right.
pub fn segment_x(phase: f32, width: f32) -> f32 {
    -SEGMENT_SHARE * width + phase * (1.0 + SEGMENT_SHARE) * width
}

/// Description of a progress bar.
pub struct ProgressBar {
    /// Track colour.
    pub track: Signal<Color>,
    /// Filled part and segment colour.
    pub fill: Signal<Color>,
    /// Progress from 0 to 1; the fill eases to it.
    pub value: Signal<f32>,
    /// Shows a sliding segment instead of the value while true.
    pub indeterminate: Signal<bool>,
    /// Width of the bar.
    pub width: Signal<f32>,
    /// Height of the bar; the ends are rounded.
    pub height: f32,
}

/// Builds the bar under `parent` and returns it; the derived signals live in `keep`.
pub fn progress_bar(scene: &mut Scene, parent: NodeId, keep: &mut Keep, bar: ProgressBar) -> NodeId {
    let ProgressBar { track, fill, value, indeterminate, width, height } = bar;
    let eased = keep.ease(&value, FAST, Easing::Linear);
    let phase = Signal::new(0.0_f32);
    keep.looping(&indeterminate, &phase, 1.0, INDETERMINATE_MS);
    let fill_w = {
        let (indeterminate, width) = (indeterminate.clone(), width.clone());
        keep.derive(move || {
            if indeterminate.get() { SEGMENT_SHARE * width.get() } else { fill_width(eased.get(), width.get()) }
        })
    };
    let fill_x = {
        let (indeterminate, width) = (indeterminate, width.clone());
        keep.derive(move || if indeterminate.get() { segment_x(phase.get(), width.get()) } else { 0.0 })
    };
    let bar =
        scene.add(parent, Node::rect().color(&track).radius(height / 2.0).clip(true).width(&width).height(height));
    scene.add(bar, Node::rect().color(&fill).radius(height / 2.0).x(&fill_x).width(&fill_w).height(height));
    bar
}

/// Description of a spinner.
pub struct Spinner {
    /// Arc colour.
    pub color: Signal<Color>,
    /// Width and height.
    pub size: f32,
    /// Stroke width of the arc.
    pub thickness: f32,
    /// Turns while true.
    pub running: Signal<bool>,
}

/// Builds the spinner under `parent` and returns it; the turning angle lives in `keep`.
pub fn spinner(scene: &mut Scene, parent: NodeId, keep: &mut Keep, spinner: Spinner) -> NodeId {
    let Spinner { color, size, thickness, running } = spinner;
    let angle = Signal::new(0.0_f32);
    keep.looping(&running, &angle, 360.0, SPINNER_TURN_MS);
    let radius = (size - thickness) / 2.0;
    scene.add(
        parent,
        Node::canvas(move |painting| {
            let center = Vec2::new(size / 2.0, size / 2.0);
            painting.stroke(&Path::new().arc(center, radius, 0.0, TAU * 0.75, false), color.get(), thickness);
        })
        .size(size, size)
        .rotation(&angle),
    )
}

/// The progress bar as a [`Component`] named `ProgressBar`.
pub struct ProgressBarComponent;

impl Component for ProgressBarComponent {
    fn name(&self) -> &'static str {
        "ProgressBar"
    }

    fn props(&self) -> &'static [PropSpec] {
        const PROPS: &[PropSpec] = &[
            PropSpec::number("value", || Prop::number(0.0)),
            PropSpec::bool("indeterminate", || Prop::bool(false)),
            PropSpec::number("width", || Prop::number(200.0)),
            PropSpec::scaled("height", "spacing.sm", || Prop::number(6.0)),
            PropSpec::themed("track", "surface1"),
            PropSpec::themed("fill", "primary"),
        ];
        PROPS
    }

    fn build(&self, scene: &mut Scene, parent: NodeId, props: &Props) -> NodeId {
        let mut keep = Keep::default();
        let bar = progress_bar(
            scene,
            parent,
            &mut keep,
            ProgressBar {
                track: props.color("track"),
                fill: props.color("fill"),
                value: props.number("value"),
                indeterminate: props.bool("indeterminate"),
                width: props.number("width"),
                height: props.number("height").get_untracked(),
            },
        );
        if let Some(node) = scene.get_mut(bar) {
            node.keep(keep);
        }
        bar
    }
}

/// The spinner as a [`Component`] named `Spinner`.
pub struct SpinnerComponent;

impl Component for SpinnerComponent {
    fn name(&self) -> &'static str {
        "Spinner"
    }

    fn props(&self) -> &'static [PropSpec] {
        const PROPS: &[PropSpec] = &[
            PropSpec::number("size", || Prop::number(24.0)),
            PropSpec::number("thickness", || Prop::number(3.0)),
            PropSpec::bool("running", || Prop::bool(true)),
            PropSpec::themed("color", "primary"),
        ];
        PROPS
    }

    fn build(&self, scene: &mut Scene, parent: NodeId, props: &Props) -> NodeId {
        let mut keep = Keep::default();
        let node = spinner(
            scene,
            parent,
            &mut keep,
            Spinner {
                color: props.color("color"),
                size: props.number("size").get_untracked(),
                thickness: props.number("thickness").get_untracked(),
                running: props.bool("running"),
            },
        );
        if let Some(n) = scene.get_mut(node) {
            n.keep(keep);
        }
        node
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::component::Registry;

    #[test]
    fn fill_clamps_and_the_segment_crosses_the_bar() {
        assert_eq!(fill_width(0.5, 100.0), 50.0);
        assert_eq!(fill_width(2.0, 100.0), 100.0);
        assert_eq!(fill_width(-1.0, 100.0), 0.0);
        assert_eq!(segment_x(0.0, 100.0), -40.0);
        assert_eq!(segment_x(1.0, 100.0), 100.0);
    }

    #[test]
    fn the_registry_builds_both_and_the_fill_follows_the_value() {
        let mut registry = Registry::new();
        registry.register(ProgressBarComponent);
        registry.register(SpinnerComponent);
        let colors = |names: &[&str]| -> Vec<(String, Prop)> {
            names.iter().map(|n| ((*n).to_owned(), Prop::color(Color::hex(0x778899)))).collect()
        };
        let value = Signal::new(0.5_f32);
        let mut given = colors(&["track", "fill"]);
        given.push(("value".to_owned(), Prop::Number(value.clone())));
        let mut scene = Scene::new(acanthus_scene::Fonts::new());
        let root = scene.root();
        let bar = registry.build("ProgressBar", &mut scene, root, given).unwrap();
        scene.frame();
        let fill = scene.get(bar).unwrap().children()[0];
        assert!(scene.geometry(fill).is_some_and(|g| g.w > 0.0 && g.w <= 100.0));
        value.set(0.0);
        for _ in 0..40 {
            acanthus_anim::tick(0.016);
            scene.frame();
        }
        assert_eq!(scene.geometry(fill).map_or(1.0, |g| g.w), 0.0);
        let spinner = registry.build("Spinner", &mut scene, root, colors(&["color"])).unwrap();
        scene.frame();
        assert!(scene.geometry(spinner).is_some_and(|g| g.w == 24.0));
    }
}
