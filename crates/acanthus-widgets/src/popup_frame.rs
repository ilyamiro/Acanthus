//! Popup frame: the card every popup body sits in, with a rounded fill, an outline, a drop shadow and an open and close morph.

use std::rc::Rc;

use acanthus_anim::{Easing, Keep, Tween, start};
use acanthus_core::{Color, Effect, Signal, Vec2, untrack};
use acanthus_scene::{LayerEffect, Node, NodeId, Scene, TransformOrigin};

use crate::component::{Component, Prop, PropSpec, Props};
use crate::dropdown::{ENTER_MS, EXIT_MS, Run};

/// Description of a popup frame.
pub struct PopupFrameSpec {
    /// Whether the popup is showing; the card morphs in and out as it changes.
    pub open: Signal<bool>,
    /// Width of the card.
    pub width: Signal<f32>,
    /// Height of the card.
    pub height: Signal<f32>,
    /// Corner radius.
    pub radius: Signal<f32>,
    /// Card fill.
    pub fill: Signal<Color>,
    /// Outline colour.
    pub border: Signal<Color>,
    /// Shadow colour; fully transparent turns the shadow off.
    pub shadow: Color,
    /// Blur radius of the shadow; it falls a third of it below the card.
    pub shadow_blur: f32,
    /// Edge or corner the card grows from while it scales in.
    pub origin: TransformOrigin,
    /// Distance the card travels downwards while it opens.
    pub slide: f32,
}

/// The signals of an open and close morph: the card fades, scales from 0.92 and slides while `presence` runs from 0 to 1.
pub(crate) struct Morph {
    pub(crate) presence: Signal<f32>,
    pub(crate) visible: Signal<bool>,
    pub(crate) scale: Signal<f32>,
    pub(crate) y: Signal<f32>,
}

pub(crate) fn morph(keep: &mut Keep, open: &Signal<bool>, slide: f32) -> Morph {
    let presence = Signal::new(0.0_f32);
    let run = Rc::new(Run::default());
    {
        let (open, presence, run) = (open.clone(), presence.clone(), run.clone());
        keep.add(Effect::new(move || {
            let on = open.get();
            untrack(|| {
                let (target, ms, easing) =
                    if on { (1.0, ENTER_MS, Easing::OutQuint) } else { (0.0, EXIT_MS, Easing::InQuint) };
                run.set(start(Tween::new(&presence, target, ms, easing)));
            });
        }));
    }
    keep.add(run);
    let visible = {
        let presence = presence.clone();
        keep.derive(move || presence.get() > 0.0)
    };
    let scale = {
        let presence = presence.clone();
        keep.derive(move || 0.92 + 0.08 * presence.get())
    };
    let y = {
        let presence = presence.clone();
        keep.derive(move || (presence.get() - 1.0) * slide)
    };
    Morph { presence, visible, scale, y }
}

/// Builds the card under `parent` and returns it; the caller adds the popup body (and, first, a wallpaper backdrop) as its children.
pub fn popup_frame(scene: &mut Scene, parent: NodeId, spec: PopupFrameSpec) -> NodeId {
    let mut keep = Keep::default();
    let Morph { presence, visible, scale, y } = morph(&mut keep, &spec.open, spec.slide);
    let shadow = LayerEffect::shadow(spec.shadow, 1.0, spec.shadow_blur, Vec2::new(0.0, spec.shadow_blur / 3.0));
    let card = scene.add(
        parent,
        Node::rect()
            .color(&spec.fill)
            .radius(&spec.radius)
            .border(1.0, &spec.border)
            .width(&spec.width)
            .height(&spec.height)
            .opacity(&presence)
            .visible(&visible)
            .scale(&scale)
            .y(&y)
            .transform_origin(spec.origin)
            .layer_effect(shadow),
    );
    if let Some(node) = scene.get_mut(card) {
        node.keep(keep);
    }
    card
}

/// The popup frame as a [`Component`] named `PopupFrame`.
pub struct PopupFrameComponent;

impl Component for PopupFrameComponent {
    fn name(&self) -> &'static str {
        "PopupFrame"
    }

    fn props(&self) -> &'static [PropSpec] {
        const PROPS: &[PropSpec] = &[
            PropSpec::bool("open", || Prop::bool(false)),
            PropSpec::number("width", || Prop::number(320.0)),
            PropSpec::number("height", || Prop::number(240.0)),
            PropSpec::scaled("radius", "radii.lg", || Prop::number(14.0)),
            PropSpec::number("shadow_blur", || Prop::number(24.0)),
            PropSpec::number("slide", || Prop::number(8.0)),
            PropSpec::themed("fill", "base"),
            PropSpec::themed("border", "surface1"),
            PropSpec::themed("shadow", "crust"),
        ];
        PROPS
    }

    fn build(&self, scene: &mut Scene, parent: NodeId, props: &Props) -> NodeId {
        let get = |name: &str| props.number(name).get_untracked();
        popup_frame(
            scene,
            parent,
            PopupFrameSpec {
                open: props.bool("open"),
                width: props.number("width"),
                height: props.number("height"),
                radius: props.number("radius"),
                fill: props.color("fill"),
                border: props.color("border"),
                shadow: props.color("shadow").get_untracked(),
                shadow_blur: get("shadow_blur"),
                origin: TransformOrigin::Top,
                slide: get("slide"),
            },
        )
    }
}

#[cfg(test)]
mod tests {
    use acanthus_scene::Fonts;

    use super::*;
    use crate::component::Registry;

    #[test]
    fn the_card_morphs_in_on_open_and_out_on_close() {
        let mut registry = Registry::new();
        registry.register(PopupFrameComponent);
        let open = Signal::new(false);
        let mut given: Vec<(String, Prop)> =
            ["fill", "border", "shadow"].map(|name| (name.to_owned(), Prop::color(Color::hex(0x223344)))).to_vec();
        given.push(("open".to_owned(), Prop::Bool(open.clone())));
        let mut scene = Scene::new(Fonts::new());
        let root = scene.root();
        let card = registry.build("PopupFrame", &mut scene, root, given).unwrap();
        scene.set_viewport(Vec2::new(500.0, 400.0), 1.0);
        scene.frame();
        let state = |scene: &Scene| {
            let node = scene.get(card).unwrap();
            (node.visible.get_untracked(), node.opacity.get_untracked(), node.y.get_untracked())
        };
        assert!(!state(&scene).0);
        open.set(true);
        acanthus_anim::tick(0.1);
        scene.frame();
        let (shown, opacity, y) = state(&scene);
        assert!(shown && opacity > 0.0 && opacity < 1.0 && y < 0.0 && y > -8.0);
        acanthus_anim::tick(0.3);
        scene.frame();
        assert_eq!(state(&scene), (true, 1.0, 0.0));
        assert_eq!(scene.geometry(card).unwrap().w, 320.0);
        open.set(false);
        acanthus_anim::tick(0.3);
        scene.frame();
        assert!(!state(&scene).0);
    }
}
