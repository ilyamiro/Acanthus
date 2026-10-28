use std::rc::Rc;

use acanthus_anim::Easing;
use acanthus_core::{Color, Signal, lighten};
use acanthus_scene::{Anchors, CursorShape, MouseArea, MouseButton, Node, NodeId, Scene};

use crate::button::{self, Feel, Press, Sound};
use crate::timing::{BASE, FAST, SLOW};
use acanthus_anim::Keep;

pub const TOGGLE: Feel = Feel {
    pop_peak: 1.05,
    pop_in_ms: 100,
    pop_out_ms: SLOW,
    flash_peak: 0.2,
    flash_ms: SLOW,
    sound: "reusables/toggle/sfx.wav",
};

pub const WIDTH: f32 = 44.0;
pub const HEIGHT: f32 = 24.0;
const INSET: f32 = 3.0;

pub struct ToggleSpec {
    pub x: Signal<f32>,
    pub y: Signal<f32>,
    pub scale: f32,
    pub corner: f32,
    pub checked: Signal<bool>,
    pub accent: Signal<Color>,
    pub base: Signal<Color>,
    pub handle_on: Signal<Color>,
    pub handle_off: Signal<Color>,
    pub enabled: Signal<bool>,
    pub sound: Sound,
    pub on_toggle: Rc<dyn Fn(bool)>,
}

pub fn toggle(scene: &mut Scene, parent: NodeId, spec: ToggleSpec) -> NodeId {
    let mut keep = Keep::default();
    let s = |value: f32| (value * spec.scale).round();
    let (width, height) = (s(WIDTH), s(HEIGHT));
    let inset = s(INSET);
    let press = Press::new(TOGGLE, spec.sound.clone());
    let act = {
        let (checked, enabled, on_toggle) = (spec.checked.clone(), spec.enabled.clone(), spec.on_toggle.clone());
        button::activate(&press, move || {
            if enabled.get_untracked() {
                on_toggle(!checked.get_untracked());
            }
        })
    };
    let area = {
        let (act, enabled) = (act.clone(), spec.enabled.clone());
        MouseArea::new().on_click(move |button| {
            if button == MouseButton::Left && enabled.get_untracked() {
                act();
            }
        })
    };
    let (hovered, pressed) = (area.hovered(), area.pressed());
    let active = {
        let (hovered, enabled) = (hovered.clone(), spec.enabled.clone());
        keep.derive(move || hovered.get() && enabled.get())
    };
    let target_scale = {
        let (pressed, active, enabled) = (pressed.clone(), active.clone(), spec.enabled.clone());
        keep.derive(move || {
            if !enabled.get() {
                1.0f32
            } else if pressed.get() {
                0.95
            } else if active.get() {
                1.04
            } else {
                1.0
            }
        })
    };
    let pop = press.scale(&mut keep, &target_scale);
    let track_color = {
        let (checked, accent, base, active) =
            (spec.checked.clone(), spec.accent.clone(), spec.base.clone(), active.clone());
        let target = keep.derive(move || {
            if checked.get() {
                accent.get()
            } else if active.get() {
                lighten(base.get(), 1.2)
            } else {
                base.get()
            }
        });
        keep.ease(&target, BASE, Easing::Linear)
    };
    let track_opacity = {
        let enabled = spec.enabled.clone();
        let target = keep.derive(move || if enabled.get() { 1.0f32 } else { 0.5 });
        keep.ease(&target, FAST, Easing::Linear)
    };
    let radius = (height / 2.0).min(spec.corner);
    let handle_h = height - inset * 2.0;
    let handle_x = {
        let checked = spec.checked.clone();
        let target = keep.derive(move || if checked.get() { width - handle_h - inset } else { inset });
        keep.ease(&target, BASE, Easing::OutBack(1.2))
    };
    let handle_color = {
        let (checked, on, off) = (spec.checked.clone(), spec.handle_on.clone(), spec.handle_off.clone());
        let target = keep.derive(move || if checked.get() { on.get() } else { off.get() });
        keep.ease(&target, FAST, Easing::Linear)
    };
    keep.add(press.clone());
    let node = button::focusable(
        Node::item()
            .x(&spec.x)
            .y(&spec.y)
            .width(width)
            .height(height)
            .opacity(&track_opacity)
            .mouse(area)
            .cursor(CursorShape::Pointer)
            .scale(&pop),
        act,
    );
    let body = scene.add(parent, node);
    scene.add(body, Node::rect().anchors(Anchors::fill(0.0)).radius(radius).color(&track_color));
    scene.add(
        body,
        Node::rect()
            .x(&handle_x)
            .y(inset)
            .size(handle_h, handle_h)
            .radius((handle_h / 2.0).min((spec.corner - 3.0).max(0.0)))
            .color(&handle_color),
    );
    button::flash(scene, body, &Signal::new(radius), &press);
    if let Some(node) = scene.get_mut(body) {
        node.keep(keep);
    }
    body
}
