use crate::component::{Component, Prop, PropSpec, Props};
use acanthus_anim::Keep;
use acanthus_core::{Color, Rect, Signal, Vec2};
use acanthus_scene::{Anchors, FillMode, LayerEffect, Node, NodeId, Scene};

pub const BLUR_MAX: f32 = 64.0;
pub const MAX_FILL_ALPHA: f32 = 0.98;

pub struct Spec {
    pub active: Signal<bool>,
    pub source: Signal<String>,
    pub screen: Signal<Vec2>,
    pub origin: Signal<Vec2>,
    pub fill_alpha: Signal<f32>,
    pub radius: Signal<f32>,
    pub radii: Signal<Option<[f32; 4]>>,
    pub scale: f32,
}

pub fn image_origin(bounds: Rect, pad: f32, surface: Vec2) -> (f32, f32) {
    (pad - bounds.x - surface.x, pad - bounds.y - surface.y)
}

pub fn padded_size(bounds: Rect, pad: f32) -> (f32, f32) {
    (bounds.w + pad * 2.0, bounds.h + pad * 2.0)
}

pub fn alpha_for(element_opacity: f32) -> f32 {
    element_opacity.min(MAX_FILL_ALPHA)
}

pub fn backdrop(scene: &mut Scene, parent: NodeId, spec: Spec) -> NodeId {
    let mut keep = Keep::default();
    let pad = (BLUR_MAX * spec.scale).round();
    let visible = {
        let (active, source) = (spec.active.clone(), spec.source.clone());
        keep.derive(move || active.get() && source.with(|s| !s.is_empty()))
    };
    let outer = Node::rect()
        .anchors(Anchors::fill(0.0))
        .color(Color::TRANSPARENT)
        .radius(&spec.radius)
        .corner_radii(&spec.radii)
        .clip(true)
        .opacity(&spec.fill_alpha)
        .visible(&visible);
    let bounds = outer.bounds();
    let outer = scene.add(parent, outer);

    let size = {
        let bounds = bounds.clone();
        keep.derive(move || padded_size(bounds.get(), pad))
    };
    let (width, height) = {
        let (a, b) = (size.clone(), size.clone());
        (keep.derive(move || a.get().0), keep.derive(move || b.get().1))
    };
    let origin = {
        let (bounds, surface) = (bounds.clone(), spec.origin.clone());
        keep.derive(move || image_origin(bounds.get(), pad, surface.get()))
    };
    let (image_x, image_y) = {
        let (a, b) = (origin.clone(), origin.clone());
        (keep.derive(move || a.get().0), keep.derive(move || b.get().1))
    };
    let (screen_w, screen_h) = {
        let (a, b) = (spec.screen.clone(), spec.screen.clone());
        (keep.derive(move || a.get().x), keep.derive(move || b.get().y))
    };
    let blurred = scene.add(
        outer,
        Node::item().x(-pad).y(-pad).width(&width).height(&height).clip(true).layer(true).layer_effect(LayerEffect {
            blur: 1.0,
            blur_max: BLUR_MAX * spec.scale,
            ..Default::default()
        }),
    );
    scene.add(
        blurred,
        Node::image(&spec.source)
            .x(&image_x)
            .y(&image_y)
            .width(&screen_w)
            .height(&screen_h)
            .fill_mode(FillMode::PreserveAspectCrop),
    );
    if let Some(node) = scene.get_mut(outer) {
        node.keep(keep);
    }
    outer
}

/// The blurred wallpaper backdrop as a [`Component`] named `WallpaperBlur`; corner radii follow `radius`.
pub struct WallpaperBlurComponent;

impl Component for WallpaperBlurComponent {
    fn name(&self) -> &'static str {
        "WallpaperBlur"
    }

    fn props(&self) -> &'static [PropSpec] {
        const PROPS: &[PropSpec] = &[
            PropSpec::bool("active", || Prop::bool(true)),
            PropSpec::text("source", || Prop::text("")),
            PropSpec::point("screen", || Prop::Point(Signal::new(Vec2::ZERO))),
            PropSpec::point("origin", || Prop::Point(Signal::new(Vec2::ZERO))),
            PropSpec::number("fill_alpha", || Prop::number(1.0)),
            PropSpec::number("radius", || Prop::number(0.0)),
            PropSpec::number("scale", || Prop::number(1.0)),
        ];
        PROPS
    }

    fn build(&self, scene: &mut Scene, parent: NodeId, props: &Props) -> NodeId {
        backdrop(
            scene,
            parent,
            Spec {
                active: props.bool("active"),
                source: props.text("source"),
                screen: props.point("screen"),
                origin: props.point("origin"),
                fill_alpha: props.number("fill_alpha"),
                radius: props.number("radius"),
                radii: Signal::new(None),
                scale: props.number("scale").get_untracked(),
            },
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_image_is_anchored_to_the_screen_not_to_the_card() {
        let card = Rect::new(1500.0, 50.0, 410.0, 580.0);
        assert_eq!(image_origin(card, 64.0, Vec2::ZERO), (64.0 - 1500.0, 64.0 - 50.0));
        assert_eq!(image_origin(card, 64.0, Vec2::new(1400.0, 10.0)), (64.0 - 2900.0, 64.0 - 60.0));
        assert_eq!(padded_size(card, 64.0), (538.0, 708.0));
    }

    #[test]
    fn the_fill_alpha_never_reaches_one() {
        assert_eq!(alpha_for(0.5), 0.5);
        assert_eq!(alpha_for(1.0), 0.98);
    }

    #[test]
    fn nested_bounds_are_surface_coordinates_and_the_image_lines_up() {
        let mut scene = Scene::new(acanthus_scene::Fonts::new());
        scene.set_viewport(Vec2::new(800.0, 600.0), 1.0);
        let root = scene.root();
        let holder = scene.add(root, Node::item().x(200.0).y(100.0).width(300.0).height(200.0));
        let active = Signal::new(true);
        let backdrop = backdrop(
            &mut scene,
            holder,
            Spec {
                active: active.clone(),
                source: Signal::new("/tmp/nothing.png".to_owned()),
                screen: Signal::new(Vec2::new(800.0, 600.0)),
                origin: Signal::new(Vec2::ZERO),
                fill_alpha: Signal::new(0.8),
                radius: Signal::new(12.0),
                radii: Signal::new(None),
                scale: 1.0,
            },
        );
        scene.frame();
        let rect = scene.geometry(backdrop).unwrap();
        assert_eq!((rect.x, rect.y, rect.w, rect.h), (200.0, 100.0, 300.0, 200.0));
        let children: Vec<NodeId> = scene.get(backdrop).unwrap().children().to_vec();
        let blurred = scene.geometry(children[0]).unwrap();
        assert_eq!((blurred.x, blurred.y, blurred.w, blurred.h), (136.0, 36.0, 428.0, 328.0));
        let image = scene.get(children[0]).unwrap().children()[0];
        let rect = scene.geometry(image).unwrap();
        assert_eq!((rect.x + 0.0, rect.y + 0.0), (blurred.x + 64.0 - 200.0, blurred.y + 64.0 - 100.0));
        assert_eq!((rect.w, rect.h), (800.0, 600.0));
    }

    #[test]
    fn an_empty_source_or_inactive_gate_hides_the_backdrop() {
        let mut scene = Scene::new(acanthus_scene::Fonts::new());
        scene.set_viewport(Vec2::new(100.0, 100.0), 1.0);
        let root = scene.root();
        let (active, source) = (Signal::new(true), Signal::new(String::new()));
        let id = backdrop(
            &mut scene,
            root,
            Spec {
                active: active.clone(),
                source: source.clone(),
                screen: Signal::new(Vec2::new(100.0, 100.0)),
                origin: Signal::new(Vec2::ZERO),
                fill_alpha: Signal::new(0.5),
                radius: Signal::new(0.0),
                radii: Signal::new(None),
                scale: 1.0,
            },
        );
        let visible = |scene: &Scene| scene.get(id).unwrap().visible.get_untracked();
        scene.frame();
        assert!(!visible(&scene));
        source.set("/tmp/x.png".to_owned());
        scene.frame();
        assert!(visible(&scene));
        active.set(false);
        scene.frame();
        assert!(!visible(&scene));
    }

    #[test]
    fn the_registry_builds_a_backdrop_with_defaults_only() {
        let mut registry = crate::component::Registry::new();
        registry.register(WallpaperBlurComponent);
        let mut scene = Scene::new(acanthus_scene::Fonts::new());
        let root = scene.root();
        let node = registry.build("WallpaperBlur", &mut scene, root, vec![]).unwrap();
        assert!(scene.get(node).is_some());
    }
}
