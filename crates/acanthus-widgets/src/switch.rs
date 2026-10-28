use crate::timing::SLOW;
use crate::timing::{BASE, FAST};
use std::cell::Cell;
use std::rc::Rc;

use acanthus_anim::Easing;
use acanthus_core::{Color, Signal, Vec2};
use acanthus_scene::{Anchors, CursorShape, Key, MouseArea, MouseButton, Node, NodeId, Scene};

use crate::button::{Feel, Press, Sound, flash};
use acanthus_anim::Animated;
use acanthus_anim::Keep;

pub const SWITCH: Feel = Feel {
    pop_peak: 1.04,
    pop_in_ms: 100,
    pop_out_ms: SLOW,
    flash_peak: 0.2,
    flash_ms: SLOW,
    sound: "reusables/switch/sfx.wav",
};

const SMALL_RADIUS: f32 = 2.0;
const HIGHLIGHT_LEAD_MS: u32 = 200;
const HIGHLIGHT_TRAIL_MS: u32 = 350;
const EDGE_REST_MS: u32 = 250;
const TEXT_MARGIN: f32 = 4.0;
const MIN_FIT: f32 = 28.0;
const FIT_PADDING: f32 = 10.0;

pub struct SwitchSpec {
    pub x: Signal<f32>,
    pub y: Signal<f32>,
    pub width: Signal<f32>,
    pub height: f32,
    pub corner: f32,
    pub font_size: Signal<f32>,
    pub min_font_size: f32,
    pub family: Signal<String>,
    pub labels: Vec<Signal<String>>,
    pub count: Signal<usize>,
    pub current: Signal<usize>,
    pub rtl: Signal<bool>,
    pub accent: Signal<Color>,
    pub base: Signal<Color>,
    pub text: Signal<Color>,
    pub active_text: Signal<Color>,
    pub visible: Signal<bool>,
    pub sound: Sound,
    pub on_select: Rc<dyn Fn(usize)>,
}

pub fn fit_width(natural: f32) -> f32 {
    MIN_FIT.max(natural.ceil() + FIT_PADDING)
}

pub fn allocate(needs: &[f32], available: f32) -> Vec<f32> {
    let count = needs.len();
    if count == 0 {
        return Vec::new();
    }
    let total: f32 = needs.iter().sum();
    let largest = needs.iter().copied().fold(0.0, f32::max);
    let available = if available <= 0.0 { 120.0f32.max(largest * count as f32) } else { available };
    let equal = available / count as f32;
    if largest <= equal {
        return vec![equal; count];
    }
    if available >= total {
        let deficit: f32 = needs.iter().filter(|n| **n > equal).map(|n| n - equal).sum();
        let surplus: f32 = needs.iter().filter(|n| **n <= equal).map(|n| equal - n).sum();
        let factor = if surplus > 0.0 { (deficit / surplus).min(1.0) } else { 0.0 };
        return needs.iter().map(|n| if *n > equal { *n } else { equal - factor * (equal - n) }).collect();
    }
    needs.iter().map(|n| if total > 0.0 { available * (n / total) } else { equal }).collect()
}

pub fn placements(widths: &[f32], total: f32, rtl: bool) -> Vec<(f32, f32)> {
    let mut at = 0.0;
    widths
        .iter()
        .map(|w| {
            let x = if rtl { total - at - w } else { at };
            at += w;
            (x, *w)
        })
        .collect()
}

pub fn fitted_size(font: f32, minimum: f32, available: f32, natural: f32) -> f32 {
    if natural <= 0.0 || natural <= available { font } else { (font * available / natural).max(minimum.min(font)) }
}

pub fn edge_radii(index: usize, count: usize, corner: f32) -> [f32; 2] {
    [if index == 0 { corner } else { SMALL_RADIUS }, if index + 1 >= count { corner } else { SMALL_RADIUS }]
}

fn physical(radii: [f32; 2], rtl: bool) -> Option<[f32; 4]> {
    let [left, right] = if rtl { [radii[1], radii[0]] } else { radii };
    Some([left, right, right, left])
}

pub fn step(current: usize, count: usize, forward: bool) -> usize {
    if forward { (current + 1).min(count.saturating_sub(1)) } else { current.saturating_sub(1) }
}

fn edge(target: &Signal<f32>, current: &Signal<usize>, rises_lead: bool) -> Animated<f32> {
    let current = current.clone();
    let previous = Cell::new(current.get_untracked());
    Animated::with(target, move |_| {
        let now = current.get_untracked();
        let before = previous.replace(now);
        if now == before {
            (EDGE_REST_MS, Easing::OutExpo)
        } else if (now > before) == rises_lead {
            (HIGHLIGHT_LEAD_MS, Easing::OutExpo)
        } else {
            (HIGHLIGHT_TRAIL_MS, Easing::OutExpo)
        }
    })
}

pub fn switch(scene: &mut Scene, parent: NodeId, spec: SwitchSpec) -> NodeId {
    let mut keep = Keep::default();
    let press = Press::new(SWITCH, spec.sound.clone());
    let pop = {
        let rest = Signal::new(1.0f32);
        press.scale(&mut keep, &rest)
    };
    let probes: Vec<(Node, Signal<Vec2>)> = spec
        .labels
        .iter()
        .map(|label| {
            let node =
                Node::text(label).font_family(&spec.family).font_size(&spec.font_size).font_weight(400).mask_source();
            let implicit = node.implicit_size();
            (node, implicit)
        })
        .collect();
    let widths = {
        let (width, count, implicit) =
            (spec.width.clone(), spec.count.clone(), probes.iter().map(|(_, i)| i.clone()).collect::<Vec<_>>());
        keep.derive(move || {
            let needs: Vec<f32> = implicit.iter().take(count.get()).map(|i| fit_width(i.get().x)).collect();
            allocate(&needs, width.get())
        })
    };
    let slots = {
        let (widths, total, rtl) = (widths.clone(), spec.width.clone(), spec.rtl.clone());
        keep.derive(move || placements(&widths.get(), total.get(), rtl.get()))
    };
    let slot = |keep: &mut Keep, current: &Signal<usize>| {
        let (slots, current) = (slots.clone(), current.clone());
        keep.derive(move || slots.with(|s| s.get(current.get()).copied().unwrap_or((0.0, 0.0))))
    };
    let current_slot = slot(&mut keep, &spec.current);
    let left_target = {
        let slot = current_slot.clone();
        keep.derive(move || slot.get().0)
    };
    let right_target = {
        let slot = current_slot;
        keep.derive(move || slot.get().0 + slot.get().1)
    };
    let left = edge(&left_target, &spec.current, false);
    let right = edge(&right_target, &spec.current, true);
    let (left_signal, right_signal) = (left.signal().clone(), right.signal().clone());
    keep.add(left);
    keep.add(right);
    let (left, right) = (left_signal, right_signal);
    let highlight_width = {
        let (left, right) = (left.clone(), right.clone());
        keep.derive(move || (right.get() - left.get()).max(0.0))
    };
    let edge_target = {
        let (current, count, corner, rtl) = (spec.current.clone(), spec.count.clone(), spec.corner, spec.rtl.clone());
        keep.derive(move || {
            let radii = edge_radii(current.get(), count.get(), corner);
            if rtl.get() { [radii[1], radii[0]] } else { radii }
        })
    };
    let left_radius = {
        let edge = edge_target.clone();
        let target = keep.derive(move || edge.get()[0]);
        keep.ease(&target, FAST, Easing::Linear)
    };
    let right_radius = {
        let target = keep.derive(move || edge_target.get()[1]);
        keep.ease(&target, FAST, Easing::Linear)
    };
    let highlight_radii = {
        let (l, r) = (left_radius, right_radius);
        keep.derive(move || Some([l.get(), r.get(), r.get(), l.get()]))
    };
    let accent = keep.ease(&spec.accent, FAST, Easing::Linear);
    let background = keep.ease(&spec.base, FAST, Easing::Linear);
    let corner = spec.corner;
    let radius = Signal::new(corner);
    keep.add(press.clone());
    let key_handler = {
        let (current, count, rtl, on_select, press) =
            (spec.current.clone(), spec.count.clone(), spec.rtl.clone(), spec.on_select.clone(), press.clone());
        move |event: &acanthus_scene::KeyEvent| {
            let forward = match event.key {
                Key::Right => !rtl.get_untracked(),
                Key::Left => rtl.get_untracked(),
                _ => return false,
            };
            let from = current.get_untracked();
            let to = step(from, count.get_untracked(), forward);
            if to != from {
                on_select(to);
                press.fire();
            }
            true
        }
    };
    let root_node = Node::item()
        .x(&spec.x)
        .y(&spec.y)
        .width(&spec.width)
        .height(spec.height)
        .visible(&spec.visible)
        .focusable(true)
        .on_key(key_handler)
        .own(keep);
    let root = scene.add(parent, root_node);
    let body = scene.add(root, Node::item().anchors(Anchors::fill(0.0)).scale(&pop));
    scene.add(body, Node::rect().anchors(Anchors::fill(0.0)).radius(corner).color(&background).clip(true));
    scene.add(
        body,
        Node::rect()
            .x(&left)
            .width(&highlight_width)
            .height(spec.height)
            .color(&accent)
            .radius(corner)
            .corner_radii(&highlight_radii),
    );
    for ((index, label), (probe, natural)) in spec.labels.iter().enumerate().zip(probes) {
        scene.add(body, probe);
        option(scene, body, &spec, &press, &slots, Slot { index, label, natural });
    }
    flash(scene, body, &radius, &press);
    root
}

struct Slot<'a> {
    index: usize,
    label: &'a Signal<String>,
    natural: Signal<Vec2>,
}

fn option(
    scene: &mut Scene,
    parent: NodeId,
    spec: &SwitchSpec,
    press: &Rc<Press>,
    slots: &Signal<Vec<(f32, f32)>>,
    cell: Slot,
) {
    let Slot { index, label, natural } = cell;
    let mut keep = Keep::default();
    let area = MouseArea::new();
    let hovered = area.hovered();
    let area = {
        let (current, on_select, press) = (spec.current.clone(), spec.on_select.clone(), press.clone());
        area.on_click(move |button| {
            if button != MouseButton::Left {
                return;
            }
            if current.get_untracked() != index {
                on_select(index);
            }
            press.fire();
        })
    };
    let x = {
        let slots = slots.clone();
        keep.derive(move || slots.with(|s| s.get(index).map_or(0.0, |slot| slot.0)))
    };
    let width = {
        let slots = slots.clone();
        keep.derive(move || slots.with(|s| s.get(index).map_or(0.0, |slot| slot.1)))
    };
    let visible = {
        let count = spec.count.clone();
        keep.derive(move || index < count.get())
    };
    let hover_color = {
        let (hovered, current) = (hovered, spec.current.clone());
        let target = keep.derive(move || {
            if hovered.get() && current.get() != index { Color::WHITE.with_alpha(0.1) } else { Color::TRANSPARENT }
        });
        keep.ease(&target, BASE, Easing::Linear)
    };
    let radii = {
        let (count, corner, rtl) = (spec.count.clone(), spec.corner, spec.rtl.clone());
        keep.derive(move || physical(edge_radii(index, count.get(), corner), rtl.get()))
    };
    let ink = {
        let (current, text, active) = (spec.current.clone(), spec.text.clone(), spec.active_text.clone());
        let target = keep.derive(move || if current.get() == index { active.get() } else { text.get() });
        keep.ease(&target, BASE, Easing::Linear)
    };
    let text_node = Node::text(label).font_family(&spec.family).font_weight(400).color(&ink).anchors(Anchors::center());
    let fitted = {
        let (font, width, natural, minimum) = (spec.font_size.clone(), width.clone(), natural, spec.min_font_size);
        keep.derive(move || fitted_size(font.get(), minimum, width.get() - TEXT_MARGIN * 2.0, natural.get().x))
    };
    let item = scene.add(
        parent,
        Node::item()
            .x(&x)
            .width(&width)
            .height(spec.height)
            .visible(&visible)
            .mouse(area)
            .cursor(CursorShape::Pointer)
            .own(keep),
    );
    scene.add(
        item,
        Node::rect().anchors(Anchors::fill(0.0)).radius(spec.corner).corner_radii(&radii).color(&hover_color),
    );
    scene.add(item, text_node.font_size(&fitted));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn short_labels_share_the_width_equally() {
        assert_eq!(allocate(&[28.0, 28.0, 28.0], 240.0), vec![80.0; 3]);
        assert_eq!(allocate(&[], 100.0), Vec::<f32>::new());
    }

    #[test]
    fn a_wide_label_borrows_from_the_narrow_ones() {
        let widths = allocate(&[150.0, 40.0, 40.0], 300.0);
        assert_eq!(widths[0], 150.0);
        assert!((widths.iter().sum::<f32>() - 300.0).abs() < 1e-3);
        assert!(widths[1] < 100.0 && widths[1] >= 40.0);
    }

    #[test]
    fn when_nothing_fits_widths_are_proportional_to_the_needs() {
        let widths = allocate(&[200.0, 100.0], 150.0);
        assert!((widths[0] - 100.0).abs() < 1e-3 && (widths[1] - 50.0).abs() < 1e-3);
    }

    #[test]
    fn fit_width_has_a_floor_and_padding() {
        assert_eq!(fit_width(5.0), 28.0);
        assert_eq!(fit_width(40.2), 51.0);
    }

    #[test]
    fn right_to_left_mirrors_the_slots() {
        assert_eq!(placements(&[80.0, 60.0], 140.0, false), vec![(0.0, 80.0), (80.0, 60.0)]);
        assert_eq!(placements(&[80.0, 60.0], 140.0, true), vec![(60.0, 80.0), (0.0, 60.0)]);
    }

    #[test]
    fn text_shrinks_to_fit_but_not_below_the_minimum() {
        assert_eq!(fitted_size(14.0, 7.0, 100.0, 80.0), 14.0);
        assert_eq!(fitted_size(14.0, 7.0, 50.0, 100.0), 7.0);
        assert!((fitted_size(14.0, 7.0, 90.0, 100.0) - 12.6).abs() < 1e-4);
        assert_eq!(fitted_size(14.0, 7.0, 10.0, 100.0), 7.0);
        assert_eq!(fitted_size(5.0, 7.0, 10.0, 100.0), 5.0);
    }

    #[test]
    fn only_the_outer_options_get_the_large_radius() {
        assert_eq!(edge_radii(0, 3, 15.0), [15.0, 2.0]);
        assert_eq!(edge_radii(1, 3, 15.0), [2.0, 2.0]);
        assert_eq!(edge_radii(2, 3, 15.0), [2.0, 15.0]);
        assert_eq!(edge_radii(0, 1, 15.0), [15.0, 15.0]);
        assert_eq!(physical([15.0, 2.0], true), Some([2.0, 15.0, 15.0, 2.0]));
    }

    #[test]
    fn arrow_keys_step_and_clamp() {
        assert_eq!(step(0, 3, true), 1);
        assert_eq!(step(2, 3, true), 2);
        assert_eq!(step(0, 3, false), 0);
        assert_eq!(step(2, 3, false), 1);
    }
}
