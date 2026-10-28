//! Press feedback shared by every button: a scale pop, a white flash, an optional click sound,
//! keyboard activation, and the ready-made icon button.

use crate::component::{Component, Prop, PropSpec, Props};
use crate::timing::{BASE, FAST, SLOW};
use std::cell::RefCell;
use std::rc::Rc;

use acanthus_anim::{AnimHandle, Easing, Sequential, Tween, start};
use acanthus_core::{Color, Signal, lighten};
use acanthus_scene::{Anchors, CursorShape, Key, MouseArea, MouseButton, Node, NodeId, Scene};

use acanthus_anim::Keep;

/// Optional sound player: receives the relative path of a sound asset (for example a
/// [`Feel::sound`]) and plays it. `None` keeps the widget silent.
pub type Sound = Option<Rc<dyn Fn(&str)>>;

/// Shape of a press: how far the body pops, how bright the flash gets and which sound plays.
#[derive(Clone, Copy)]
pub struct Feel {
    /// Scale reached at the top of the pop (1.0 is the resting size).
    pub pop_peak: f32,
    /// Duration of the pop towards its peak, in milliseconds (OutQuad).
    pub pop_in_ms: u32,
    /// Duration of the return from the peak to 1.0, in milliseconds (OutQuint).
    pub pop_out_ms: u32,
    /// Opacity of the white flash right after the press.
    pub flash_peak: f32,
    /// Duration of the flash fade to transparent, in milliseconds (OutExpo).
    pub flash_ms: u32,
    /// Relative path of the click sound handed to the [`Sound`] player.
    pub sound: &'static str,
}

/// Feel of a text button: a light pop and a faint flash.
pub const CLICK: Feel = Feel {
    pop_peak: 1.015,
    pop_in_ms: 100,
    pop_out_ms: SLOW,
    flash_peak: 0.15,
    flash_ms: SLOW,
    sound: "reusables/clickbutton/click.wav",
};

/// Feel of an icon button: a stronger pop and flash.
pub const ICON: Feel = Feel {
    pop_peak: 1.1,
    pop_in_ms: 110,
    pop_out_ms: SLOW,
    flash_peak: 0.4,
    flash_ms: 400,
    sound: "reusables/iconbutton/click.wav",
};

/// Press animation state of one widget: drives the pop and flash signals and plays the sound.
pub struct Press {
    feel: Feel,
    sound: Sound,
    pop: Signal<f32>,
    flash: Signal<f32>,
    pop_run: RefCell<Option<AnimHandle>>,
    flash_run: RefCell<Option<AnimHandle>>,
}

impl Press {
    /// Creates the press state for a widget with the given feel and sound player.
    pub fn new(feel: Feel, sound: Sound) -> Rc<Press> {
        Rc::new(Press {
            feel,
            sound,
            pop: Signal::new(1.0),
            flash: Signal::new(0.0),
            pop_run: RefCell::default(),
            flash_run: RefCell::default(),
        })
    }

    /// Starts the pop and the flash from the beginning (restarting a running one) and plays the sound.
    pub fn fire(&self) {
        let feel = self.feel;
        self.pop.set(1.0);
        let sequence = Sequential::new()
            .then(Tween::new(&self.pop, feel.pop_peak, feel.pop_in_ms, Easing::OutQuad))
            .then(Tween::new(&self.pop, 1.0, feel.pop_out_ms, Easing::OutQuint));
        if let Some(old) = self.pop_run.borrow_mut().replace(start(sequence)) {
            old.stop();
        }
        self.flash.set(feel.flash_peak);
        let fade = Tween::new(&self.flash, 0.0, feel.flash_ms, Easing::OutExpo);
        if let Some(old) = self.flash_run.borrow_mut().replace(start(fade)) {
            old.stop();
        }
        if let Some(sound) = &self.sound {
            sound(feel.sound);
        }
    }

    /// Returns the eased body scale: `hover` (the hover or press target scale) multiplied by the pop,
    /// smoothed over the base duration. The intermediate signals live in `keep`.
    pub fn scale(&self, keep: &mut Keep, hover: &Signal<f32>) -> Signal<f32> {
        let (hover, pop) = (hover.clone(), self.pop.clone());
        let target = keep.derive(move || hover.get() * pop.get());
        keep.ease(&target, BASE, Easing::OutQuint)
    }
}

impl Drop for Press {
    fn drop(&mut self) {
        for slot in [&self.pop_run, &self.flash_run] {
            if let Some(handle) = slot.borrow_mut().take() {
                handle.stop();
            }
        }
    }
}

/// Wraps a click handler so that every activation fires `press` first.
pub fn activate(press: &Rc<Press>, click: impl Fn() + 'static) -> Rc<dyn Fn()> {
    let press = press.clone();
    Rc::new(move || {
        press.fire();
        click();
    })
}

/// Makes `node` focusable and activates it with Space or Enter.
pub fn focusable(node: Node, activate: Rc<dyn Fn()>) -> Node {
    node.focusable(true).on_key(move |event| {
        if matches!(event.key, Key::Char(' ') | Key::Enter) {
            activate();
            true
        } else {
            false
        }
    })
}

/// Feel of a filled button: the body shrinks briefly instead of growing, with a bright flash.
pub const FILL: Feel = Feel {
    pop_peak: 0.95,
    pop_in_ms: 110,
    pop_out_ms: 400,
    flash_peak: 0.5,
    flash_ms: 400,
    sound: "reusables/fillbutton/button.wav",
};

/// Adds the white flash rectangle of `press` over `body`, clipped to `radius`.
pub fn flash(scene: &mut Scene, body: NodeId, radius: &Signal<f32>, press: &Press) {
    scene.add(body, Node::rect().anchors(Anchors::fill(0.0)).radius(radius).color(Color::WHITE).opacity(&press.flash));
}

/// Adds the press decorations (currently the flash) to a button body.
pub fn dress(scene: &mut Scene, body: NodeId, radius: &Signal<f32>, press: &Press) {
    flash(scene, body, radius, press);
}

/// Description of an icon button: size, glyph, colours, sound and click handler.
pub struct IconSpec {
    /// Width in logical pixels.
    pub width: f32,
    /// Height in logical pixels.
    pub height: f32,
    /// Corner radius of the background.
    pub radius: Signal<f32>,
    /// Glyph shown in the middle, usually a character of an icon font.
    pub glyph: Signal<String>,
    /// Font size of the glyph.
    pub glyph_size: f32,
    /// Font family of the glyph.
    pub font: Signal<String>,
    /// Background colour; lightened on hover and darkened while pressed.
    pub accent: Signal<Color>,
    /// Opacity of the background rectangle (0 gives a bare glyph).
    pub bg_opacity: Signal<f32>,
    /// Glyph colour at rest.
    pub text: Signal<Color>,
    /// Glyph colour while hovered.
    pub text_hover: Signal<Color>,
    /// Sound player for the click.
    pub sound: Sound,
    /// Called on a left click, Space or Enter.
    pub on_click: Rc<dyn Fn()>,
}

/// Builds an icon button under `parent` and returns its outer node: hover and press scale,
/// colour easing, flash, keyboard activation and a pointer cursor.
pub fn icon_button(scene: &mut Scene, parent: NodeId, spec: IconSpec) -> NodeId {
    let mut keep = Keep::default();
    let area = MouseArea::new();
    let (hovered, pressed) = (area.hovered(), area.pressed());
    let press = Press::new(ICON, spec.sound);
    let act = activate(&press, {
        let on_click = spec.on_click;
        move || on_click()
    });
    let area = {
        let act = act.clone();
        area.on_click(move |btn| {
            if btn == MouseButton::Left {
                act()
            }
        })
    };
    let scale_target = {
        let (hovered, pressed) = (hovered.clone(), pressed.clone());
        keep.derive(move || {
            if pressed.get() {
                0.96f32
            } else if hovered.get() {
                1.04
            } else {
                1.0
            }
        })
    };
    let scale = press.scale(&mut keep, &scale_target);
    let bg_target = {
        let (hovered, pressed, accent) = (hovered.clone(), pressed, spec.accent.clone());
        keep.derive(move || {
            let base = accent.get();
            if pressed.get() {
                Color { r: base.r / 1.12, g: base.g / 1.12, b: base.b / 1.12, a: base.a }
            } else if hovered.get() {
                lighten(base, 1.12)
            } else {
                base
            }
        })
    };
    let bg = keep.ease(&bg_target, FAST, Easing::Linear);
    let glyph_color = {
        let (hovered, normal, hover) = (hovered, spec.text.clone(), spec.text_hover.clone());
        keep.derive(move || if hovered.get() { hover.get() } else { normal.get() })
    };
    let radius = spec.radius;
    keep.add(press.clone());
    let outer = focusable(
        Node::item().width(spec.width).height(spec.height).mouse(area).cursor(CursorShape::Pointer).own(keep),
        act,
    );
    let outer = scene.add(parent, outer);
    let body = scene.add(outer, Node::item().anchors(Anchors::fill(0.0)).scale(&scale));
    scene.add(body, Node::rect().anchors(Anchors::fill(0.0)).radius(&radius).color(&bg).opacity(&spec.bg_opacity));
    scene.add(
        body,
        Node::text(&spec.glyph)
            .anchors(Anchors::center())
            .font_family(&spec.font)
            .font_size(spec.glyph_size)
            .color(&glyph_color),
    );
    dress(scene, body, &radius, &press);
    outer
}

/// The icon button as a [`Component`] named `IconButton`; clicks make no sound.
pub struct IconButtonComponent;

impl Component for IconButtonComponent {
    fn name(&self) -> &'static str {
        "IconButton"
    }

    fn props(&self) -> &'static [PropSpec] {
        const PROPS: &[PropSpec] = &[
            PropSpec::number("width", || Prop::number(32.0)),
            PropSpec::number("height", || Prop::number(32.0)),
            PropSpec::scaled("radius", "radii.md", || Prop::number(8.0)),
            PropSpec::text("glyph", || Prop::text("")),
            PropSpec::number("glyph_size", || Prop::number(16.0)),
            PropSpec::text("font", || Prop::text("")),
            PropSpec::number("bg_opacity", || Prop::number(1.0)),
            PropSpec::themed("accent", "primary"),
            PropSpec::themed("text", "text"),
            PropSpec::themed("text_hover", "base"),
            PropSpec::event("on_click"),
        ];
        PROPS
    }

    fn build(&self, scene: &mut Scene, parent: NodeId, props: &Props) -> NodeId {
        icon_button(
            scene,
            parent,
            IconSpec {
                width: props.number("width").get_untracked(),
                height: props.number("height").get_untracked(),
                radius: props.number("radius"),
                glyph: props.text("glyph"),
                glyph_size: props.number("glyph_size").get_untracked(),
                font: props.text("font"),
                accent: props.color("accent"),
                bg_opacity: props.number("bg_opacity"),
                text: props.color("text"),
                text_hover: props.color("text_hover"),
                sound: None,
                on_click: props.event("on_click"),
            },
        )
    }
}

#[cfg(test)]
mod component_tests {
    use super::*;
    use crate::component::Registry;

    #[test]
    fn the_registry_builds_an_icon_button_and_runs_its_click() {
        let mut registry = Registry::new();
        registry.register(IconButtonComponent);
        let clicks = Rc::new(std::cell::Cell::new(0));
        let sink = clicks.clone();
        let mut given: Vec<(String, Prop)> =
            ["accent", "text", "text_hover"].map(|name| (name.to_owned(), Prop::color(Color::hex(0x445566)))).to_vec();
        given.push(("on_click".to_owned(), Prop::Event(Rc::new(move || sink.set(sink.get() + 1)))));
        given.push(("glyph".to_owned(), Prop::text("x")));
        let mut scene = Scene::new(acanthus_scene::Fonts::new());
        let root = scene.root();
        let node = registry.build("IconButton", &mut scene, root, given).unwrap();
        assert!(scene.get(node).is_some());
        assert_eq!(clicks.get(), 0);
    }
}
