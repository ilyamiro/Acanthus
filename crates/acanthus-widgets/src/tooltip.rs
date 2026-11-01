//! Tooltip: a small bubble centred under its target that fades in once the pointer has rested on the target for a delay.

use std::cell::RefCell;

use acanthus_anim::{Action, AnimHandle, Easing, Keep, Pause, Sequential, start};
use acanthus_core::{Color, Effect, Signal, untrack};
use acanthus_scene::{Anchors, Node, NodeId, Scene};

use crate::component::{Component, Prop, PropSpec, Props};
use crate::timing::FAST;

/// Description of a tooltip.
pub struct TooltipSpec {
    /// Text of the bubble.
    pub text: Signal<String>,
    /// True while the pointer rests on the target; the caller takes it from the target's mouse area.
    pub hovered: Signal<bool>,
    /// Milliseconds the target must stay hovered before the bubble shows.
    pub delay_ms: u32,
    /// Width of the target, for centring.
    pub target_width: Signal<f32>,
    /// Height of the target; the bubble sits `gap` below it.
    pub target_height: Signal<f32>,
    /// Space between the target and the bubble.
    pub gap: f32,
    /// Padding between the bubble edge and its text.
    pub pad: f32,
    /// Corner radius of the bubble.
    pub radius: f32,
    /// Bubble fill.
    pub fill: Signal<Color>,
    /// Bubble outline.
    pub border: Signal<Color>,
    /// Text colour.
    pub color: Signal<Color>,
    /// Text font family.
    pub font_family: Signal<String>,
    /// Text font size.
    pub font_size: f32,
}

struct Pending(RefCell<Option<AnimHandle>>);

impl Pending {
    fn stop(&self) {
        if let Some(handle) = self.0.borrow_mut().take() {
            handle.stop();
        }
    }
}

impl Drop for Pending {
    fn drop(&mut self) {
        self.stop();
    }
}

/// Builds the bubble under the target `parent` and returns it; it hides at once when the target is left.
pub fn tooltip(scene: &mut Scene, parent: NodeId, spec: TooltipSpec) -> NodeId {
    let TooltipSpec {
        text,
        hovered,
        delay_ms,
        target_width,
        target_height,
        gap,
        pad,
        radius,
        fill,
        border,
        color,
        font_family,
        font_size,
    } = spec;
    let mut keep = Keep::default();
    let shown = Signal::new(false);
    let pending = std::rc::Rc::new(Pending(RefCell::default()));
    keep.add(Effect::new({
        let (shown, pending) = (shown.clone(), pending.clone());
        move || {
            let on = hovered.get();
            untrack(|| {
                pending.stop();
                if on {
                    let shown = shown.clone();
                    let reveal =
                        Sequential::new().then(Pause::new(delay_ms)).then(Action::new(move || shown.set(true)));
                    *pending.0.borrow_mut() = Some(start(reveal));
                } else {
                    shown.set(false);
                }
            });
        }
    }));
    keep.add(pending);
    let opacity = {
        let target = keep.derive({
            let shown = shown.clone();
            move || if shown.get() { 1.0_f32 } else { 0.0 }
        });
        keep.ease(&target, FAST, Easing::Linear)
    };
    let visible = {
        let opacity = opacity.clone();
        keep.derive(move || opacity.get() > 0.0)
    };
    let label = Node::text(&text).font_family(&font_family).font_size(font_size).color(&color);
    let measured = label.implicit_size();
    let width = {
        let measured = measured.clone();
        keep.derive(move || measured.get().x + pad * 2.0)
    };
    let height = keep.derive(move || measured.get().y + pad * 2.0);
    let x = {
        let width = width.clone();
        keep.derive(move || (target_width.get() - width.get()) / 2.0)
    };
    let y = keep.derive(move || target_height.get() + gap);
    let bubble = scene.add(
        parent,
        Node::rect()
            .color(&fill)
            .border(1.0, &border)
            .radius(radius)
            .x(&x)
            .y(&y)
            .width(&width)
            .height(&height)
            .opacity(&opacity)
            .visible(&visible),
    );
    scene.add(bubble, label.anchors(Anchors::center()));
    if let Some(node) = scene.get_mut(bubble) {
        node.keep(keep);
    }
    bubble
}

/// The tooltip as a [`Component`] named `Tooltip`.
pub struct TooltipComponent;

impl Component for TooltipComponent {
    fn name(&self) -> &'static str {
        "Tooltip"
    }

    fn props(&self) -> &'static [PropSpec] {
        const PROPS: &[PropSpec] = &[
            PropSpec::text("text", || Prop::text("")),
            PropSpec::bool("hovered", || Prop::bool(false)),
            PropSpec::number("delay", || Prop::number(700.0)),
            PropSpec::number("target_width", || Prop::number(0.0)),
            PropSpec::number("target_height", || Prop::number(0.0)),
            PropSpec::scaled("gap", "spacing.sm", || Prop::number(6.0)),
            PropSpec::scaled("pad", "spacing.sm", || Prop::number(8.0)),
            PropSpec::scaled("radius", "radii.sm", || Prop::number(6.0)),
            PropSpec::text("font_family", || Prop::text("")),
            PropSpec::number("font_size", || Prop::number(12.0)),
            PropSpec::themed("fill", "mantle"),
            PropSpec::themed("border", "surface2"),
            PropSpec::themed("color", "text"),
        ];
        PROPS
    }

    fn build(&self, scene: &mut Scene, parent: NodeId, props: &Props) -> NodeId {
        tooltip(
            scene,
            parent,
            TooltipSpec {
                text: props.text("text"),
                hovered: props.bool("hovered"),
                delay_ms: props.number("delay").get_untracked().max(0.0) as u32,
                target_width: props.number("target_width"),
                target_height: props.number("target_height"),
                gap: props.number("gap").get_untracked(),
                pad: props.number("pad").get_untracked(),
                radius: props.number("radius").get_untracked(),
                fill: props.color("fill"),
                border: props.color("border"),
                color: props.color("color"),
                font_family: props.text("font_family"),
                font_size: props.number("font_size").get_untracked(),
            },
        )
    }
}

#[cfg(test)]
mod tests {
    use acanthus_core::Vec2;
    use acanthus_scene::Fonts;

    use super::*;
    use crate::component::Registry;

    #[test]
    fn the_bubble_shows_after_the_delay_and_hides_when_the_target_is_left() {
        let mut registry = Registry::new();
        registry.register(TooltipComponent);
        let hovered = Signal::new(false);
        let mut given: Vec<(String, Prop)> =
            ["fill", "border", "color"].map(|name| (name.to_owned(), Prop::color(Color::hex(0x334455)))).to_vec();
        given.push(("text".to_owned(), Prop::text("Mute")));
        given.push(("font_family".to_owned(), Prop::text("Adwaita Mono")));
        given.push(("hovered".to_owned(), Prop::Bool(hovered.clone())));
        given.push(("target_width".to_owned(), Prop::number(40.0)));
        given.push(("target_height".to_owned(), Prop::number(30.0)));
        let mut scene = Scene::new(Fonts::new());
        let root = scene.root();
        let bubble = registry.build("Tooltip", &mut scene, root, given).unwrap();
        scene.set_viewport(Vec2::new(300.0, 200.0), 1.0);
        let shown = |scene: &Scene| scene.get(bubble).unwrap().visible.get_untracked();
        scene.frame();
        assert!(!shown(&scene));
        hovered.set(true);
        acanthus_anim::tick(0.5);
        scene.frame();
        assert!(!shown(&scene));
        acanthus_anim::tick(0.3);
        acanthus_anim::tick(0.05);
        scene.frame();
        assert!(shown(&scene));
        let at = scene.geometry(bubble).unwrap();
        assert_eq!(at.y, 36.0);
        hovered.set(false);
        for _ in 0..20 {
            acanthus_anim::tick(0.016);
            scene.frame();
        }
        assert!(!shown(&scene));
    }

    #[test]
    fn leaving_before_the_delay_cancels_the_reveal() {
        let hovered = Signal::new(true);
        let mut scene = Scene::new(Fonts::new());
        let root = scene.root();
        let bubble = tooltip(
            &mut scene,
            root,
            TooltipSpec {
                text: Signal::new("x".to_owned()),
                hovered: hovered.clone(),
                delay_ms: 500,
                target_width: Signal::new(10.0),
                target_height: Signal::new(10.0),
                gap: 4.0,
                pad: 4.0,
                radius: 4.0,
                fill: Signal::new(Color::BLACK),
                border: Signal::new(Color::BLACK),
                color: Signal::new(Color::WHITE),
                font_family: Signal::new("Adwaita Mono".to_owned()),
                font_size: 12.0,
            },
        );
        acanthus_anim::tick(0.3);
        hovered.set(false);
        acanthus_anim::tick(0.5);
        for _ in 0..10 {
            acanthus_anim::tick(0.016);
        }
        assert!(!scene.get(bubble).unwrap().visible.get_untracked());
    }
}
