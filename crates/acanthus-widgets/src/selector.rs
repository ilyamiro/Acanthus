//! Number selector and time selector: a value between a minus and a plus button, changed by click, wheel and keys.

use std::cell::RefCell;
use std::rc::Rc;

use acanthus_anim::{Easing, Keep};
use acanthus_core::{Color, Signal};
use acanthus_scene::{Anchors, CursorShape, Key, MouseArea, MouseButton, Node, NodeId, Scene};

use crate::component::{Component, Prop, PropSpec, Props};
use crate::timing::FAST;
use crate::wheel::{Wheel, wheel_units};

/// Colours and metrics shared by [`number_selector`] and [`time_selector`].
#[derive(Clone)]
pub struct SelectorStyle {
    /// Fill of the buttons and the value box.
    pub fill: Signal<Color>,
    /// Fill of a button under the pointer.
    pub hover: Signal<Color>,
    /// Outline of the value box.
    pub border: Signal<Color>,
    /// Colour of the value and the button glyphs.
    pub text: Signal<Color>,
    /// Font family.
    pub font_family: Signal<String>,
    /// Font size.
    pub font_size: f32,
    /// Height of the control; the buttons are squares of this size.
    pub height: f32,
    /// Width of the value box.
    pub value_width: f32,
    /// Corner radius.
    pub radius: f32,
}

/// Value `steps` steps away from `value`, clamped to `min..=max`, or wrapped round when `wrap` is set.
pub fn step_value(value: f32, steps: i32, min: f32, max: f32, step: f32, wrap: bool) -> f32 {
    let moved = value + steps as f32 * step;
    if !wrap {
        return moved.clamp(min, max);
    }
    let count = ((max - min) / step).round() + 1.0;
    let slot = ((moved - min) / step).round().rem_euclid(count);
    min + slot * step
}

/// The value as text with `decimals` places and at least `digits` integer digits.
pub fn format_value(value: f32, decimals: usize, digits: usize) -> String {
    let text = format!("{:.*}", decimals, value.abs());
    let whole = text.split('.').next().map_or(0, str::len);
    let sign = if value < 0.0 && text.chars().any(|c| c != '0' && c != '.') { "-" } else { "" };
    format!("{sign}{}{text}", "0".repeat(digits.saturating_sub(whole)))
}

/// Number of steps a key press asks for, or the bound it jumps to.
enum Move {
    Steps(i32),
    Min,
    Max,
}

fn key_move(key: Key) -> Option<Move> {
    match key {
        Key::Up | Key::Right => Some(Move::Steps(1)),
        Key::Down | Key::Left => Some(Move::Steps(-1)),
        Key::PageUp => Some(Move::Steps(10)),
        Key::PageDown => Some(Move::Steps(-10)),
        Key::Home => Some(Move::Min),
        Key::End => Some(Move::Max),
        _ => None,
    }
}

/// Description of a number selector.
pub struct NumberSelector {
    /// Colours and metrics.
    pub style: SelectorStyle,
    /// Current value; the caller stores what `on_change` reports.
    pub value: Signal<f32>,
    /// Lowest value.
    pub min: f32,
    /// Highest value.
    pub max: f32,
    /// Distance between two values.
    pub step: f32,
    /// Decimal places shown.
    pub decimals: usize,
    /// Minimum integer digits shown, padded with zeros.
    pub digits: usize,
    /// Goes from the highest value to the lowest and back instead of stopping.
    pub wrap: bool,
    /// Called with the requested value.
    pub on_change: Rc<dyn Fn(f32)>,
}

fn step_button(scene: &mut Scene, parent: NodeId, glyph: &str, style: &SelectorStyle, press: Rc<dyn Fn()>) {
    let mut keep = Keep::default();
    let area = MouseArea::new().on_click(move |button| {
        if button == MouseButton::Left {
            press();
        }
    });
    let hovered = area.hovered();
    let color = {
        let (fill, hover) = (style.fill.clone(), style.hover.clone());
        let target = keep.derive(move || if hovered.get() { hover.get() } else { fill.get() });
        keep.ease(&target, FAST, Easing::Linear)
    };
    let size = style.height;
    let button = scene.add(
        parent,
        Node::rect().size(size, size).radius(style.radius).color(&color).mouse(area).cursor(CursorShape::Pointer),
    );
    scene.add(
        button,
        Node::text(glyph.to_owned())
            .font_family(&style.font_family)
            .font_size(style.font_size)
            .color(&style.text)
            .anchors(Anchors::center()),
    );
    if let Some(node) = scene.get_mut(button) {
        node.keep(keep);
    }
}

/// Builds a number selector under `parent` and returns its row.
pub fn number_selector(scene: &mut Scene, parent: NodeId, spec: NumberSelector) -> NodeId {
    let NumberSelector { style, value, min, max, step, decimals, digits, wrap, on_change } = spec;
    let mut keep = Keep::default();
    let shift: Rc<dyn Fn(i32)> = {
        let (value, on_change) = (value.clone(), on_change.clone());
        Rc::new(move |steps| on_change(step_value(value.get_untracked(), steps, min, max, step, wrap)))
    };
    let shown = {
        let value = value.clone();
        keep.derive(move || format_value(value.get(), decimals, digits))
    };
    let wheel = RefCell::new(Wheel::default());
    let area = {
        let shift = shift.clone();
        MouseArea::new().on_scroll(move |delta| {
            let notches = wheel.borrow_mut().feed(wheel_units(delta.y));
            if notches != 0 {
                shift(notches);
            }
        })
    };
    let key_shift = shift.clone();
    let key_value = on_change.clone();
    let row = scene.add(
        parent,
        Node::row(0.0).height(style.height).mouse(area).focusable(true).on_key(move |event| {
            match key_move(event.key) {
                Some(Move::Steps(steps)) => {
                    key_shift(steps);
                    true
                }
                Some(Move::Min) => {
                    key_value(min);
                    true
                }
                Some(Move::Max) => {
                    key_value(max);
                    true
                }
                None => false,
            }
        }),
    );
    step_button(scene, row, "\u{2212}", &style, {
        let shift = shift.clone();
        Rc::new(move || shift(-1))
    });
    let boxed = scene
        .add(row, Node::rect().size(style.value_width, style.height).color(&style.fill).border(1.0, &style.border));
    scene.add(
        boxed,
        Node::text(&shown)
            .font_family(&style.font_family)
            .font_size(style.font_size)
            .color(&style.text)
            .anchors(Anchors::center()),
    );
    step_button(scene, row, "+", &style, Rc::new(move || shift(1)));
    if let Some(node) = scene.get_mut(row) {
        node.keep(keep);
    }
    row
}

/// Description of a time selector.
pub struct TimeSelector {
    /// Colours and metrics.
    pub style: SelectorStyle,
    /// Minutes since midnight.
    pub minutes: Signal<f32>,
    /// Minutes between two selectable values of the minute field.
    pub minute_step: f32,
    /// Space between the two fields and the colon.
    pub gap: f32,
    /// Called with the requested minutes since midnight.
    pub on_change: Rc<dyn Fn(f32)>,
}

/// Builds an hour field, a colon and a minute field under `parent` and returns the row.
pub fn time_selector(scene: &mut Scene, parent: NodeId, spec: TimeSelector) -> NodeId {
    let TimeSelector { style, minutes, minute_step, gap, on_change } = spec;
    let mut keep = Keep::default();
    let hours = {
        let minutes = minutes.clone();
        keep.derive(move || (minutes.get() / 60.0).floor())
    };
    let rest = {
        let minutes = minutes.clone();
        keep.derive(move || minutes.get().rem_euclid(60.0))
    };
    let row = scene.add(parent, Node::row(gap).height(style.height));
    let field = |value: Signal<f32>, max: f32, step: f32, change: Rc<dyn Fn(f32)>| NumberSelector {
        style: style.clone(),
        value,
        min: 0.0,
        max,
        step,
        decimals: 0,
        digits: 2,
        wrap: true,
        on_change: change,
    };
    number_selector(
        scene,
        row,
        field(hours, 23.0, 1.0, {
            let (minutes, on_change) = (minutes.clone(), on_change.clone());
            Rc::new(move |hour| on_change(hour * 60.0 + minutes.get_untracked().rem_euclid(60.0)))
        }),
    );
    scene.add(
        row,
        Node::text(":".to_owned())
            .font_family(&style.font_family)
            .font_size(style.font_size)
            .color(&style.text)
            .anchors(Anchors::default().v_center()),
    );
    number_selector(
        scene,
        row,
        field(rest, 60.0 - minute_step, minute_step, {
            let minutes = minutes.clone();
            Rc::new(move |minute| on_change((minutes.get_untracked() / 60.0).floor() * 60.0 + minute))
        }),
    );
    if let Some(node) = scene.get_mut(row) {
        node.keep(keep);
    }
    row
}

fn style_from(props: &Props) -> SelectorStyle {
    SelectorStyle {
        fill: props.color("fill"),
        hover: props.color("hover"),
        border: props.color("border"),
        text: props.color("text"),
        font_family: props.text("font_family"),
        font_size: props.number("font_size").get_untracked(),
        height: props.number("height").get_untracked(),
        value_width: props.number("value_width").get_untracked(),
        radius: props.number("radius").get_untracked(),
    }
}

macro_rules! with_style_props {
    ($($extra:expr),* $(,)?) => {
        &[
            PropSpec::text("font_family", || Prop::text("")),
            PropSpec::number("font_size", || Prop::number(14.0)),
            PropSpec::number("height", || Prop::number(32.0)),
            PropSpec::number("value_width", || Prop::number(56.0)),
            PropSpec::scaled("radius", "radii.sm", || Prop::number(6.0)),
            PropSpec::themed("fill", "surface0"),
            PropSpec::themed("hover", "surface1"),
            PropSpec::themed("border", "surface2"),
            PropSpec::themed("text", "text"),
            $($extra),*
        ]
    };
}

/// The number selector as a [`Component`] named `NumberSelector`.
pub struct NumberSelectorComponent;

impl Component for NumberSelectorComponent {
    fn name(&self) -> &'static str {
        "NumberSelector"
    }

    fn props(&self) -> &'static [PropSpec] {
        const PROPS: &[PropSpec] = with_style_props![
            PropSpec::number("value", || Prop::number(0.0)),
            PropSpec::number("min", || Prop::number(0.0)),
            PropSpec::number("max", || Prop::number(100.0)),
            PropSpec::number("step", || Prop::number(1.0)),
            PropSpec::number("decimals", || Prop::number(0.0)),
            PropSpec::number("digits", || Prop::number(1.0)),
            PropSpec::bool("wrap", || Prop::bool(false)),
            PropSpec::number_event("on_change"),
        ];
        PROPS
    }

    fn build(&self, scene: &mut Scene, parent: NodeId, props: &Props) -> NodeId {
        let get = |name: &str| props.number(name).get_untracked();
        number_selector(
            scene,
            parent,
            NumberSelector {
                style: style_from(props),
                value: props.number("value"),
                min: get("min"),
                max: get("max"),
                step: get("step").max(f32::EPSILON),
                decimals: get("decimals").max(0.0) as usize,
                digits: get("digits").max(0.0) as usize,
                wrap: props.bool("wrap").get_untracked(),
                on_change: props.number_event("on_change"),
            },
        )
    }
}

/// The time selector as a [`Component`] named `TimeSelector`.
pub struct TimeSelectorComponent;

impl Component for TimeSelectorComponent {
    fn name(&self) -> &'static str {
        "TimeSelector"
    }

    fn props(&self) -> &'static [PropSpec] {
        const PROPS: &[PropSpec] = with_style_props![
            PropSpec::number("minutes", || Prop::number(0.0)),
            PropSpec::number("minute_step", || Prop::number(1.0)),
            PropSpec::scaled("gap", "spacing.sm", || Prop::number(8.0)),
            PropSpec::number_event("on_change"),
        ];
        PROPS
    }

    fn build(&self, scene: &mut Scene, parent: NodeId, props: &Props) -> NodeId {
        time_selector(
            scene,
            parent,
            TimeSelector {
                style: style_from(props),
                minutes: props.number("minutes"),
                minute_step: props.number("minute_step").get_untracked().clamp(1.0, 60.0),
                gap: props.number("gap").get_untracked(),
                on_change: props.number_event("on_change"),
            },
        )
    }
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;

    use acanthus_core::Vec2;
    use acanthus_scene::{Fonts, KeyEvent};

    use super::*;
    use crate::component::Registry;

    fn colors() -> Vec<(String, Prop)> {
        ["fill", "hover", "border", "text"].map(|name| (name.to_owned(), Prop::color(Color::hex(0x336699)))).to_vec()
    }

    fn click(scene: &mut Scene, at: Vec2) {
        scene.pointer_motion_at(at, 0.0);
        scene.pointer_button_at(MouseButton::Left, true, 0.0);
        scene.pointer_button_at(MouseButton::Left, false, 0.0);
    }

    #[test]
    fn values_clamp_or_wrap_and_format_with_padding() {
        assert_eq!(step_value(9.0, 3, 0.0, 10.0, 1.0, false), 10.0);
        assert_eq!(step_value(1.0, -3, 0.0, 10.0, 1.0, false), 0.0);
        assert_eq!(step_value(23.0, 1, 0.0, 23.0, 1.0, true), 0.0);
        assert_eq!(step_value(0.0, -1, 0.0, 55.0, 5.0, true), 55.0);
        assert_eq!(format_value(7.0, 0, 2), "07");
        assert_eq!(format_value(2.5, 1, 1), "2.5");
        assert_eq!(format_value(-1.25, 2, 1), "-1.25");
        assert_eq!(format_value(-0.0, 0, 1), "0");
    }

    #[test]
    fn buttons_keys_and_bounds_report_the_requested_value() {
        let mut registry = Registry::new();
        registry.register(NumberSelectorComponent);
        let seen = Rc::new(Cell::new(None));
        let sink = seen.clone();
        let mut given = colors();
        given.push(("value".to_owned(), Prop::number(5.0)));
        given.push(("max".to_owned(), Prop::number(8.0)));
        given.push(("font_family".to_owned(), Prop::text("Adwaita Mono")));
        given.push(("on_change".to_owned(), Prop::NumberEvent(Rc::new(move |v| sink.set(Some(v))))));
        let mut scene = Scene::new(Fonts::new());
        let root = scene.root();
        let row = registry.build("NumberSelector", &mut scene, root, given).unwrap();
        scene.set_viewport(Vec2::new(300.0, 100.0), 1.0);
        scene.frame();
        click(&mut scene, Vec2::new(5.0, 5.0));
        assert_eq!(seen.get(), Some(4.0));
        let plus = scene.get(row).unwrap().children()[2];
        let at = scene.geometry(plus).unwrap();
        click(&mut scene, Vec2::new(at.x + 5.0, at.y + 5.0));
        assert_eq!(seen.get(), Some(6.0));
        scene.set_focus(Some(row));
        assert!(scene.key_event(&KeyEvent::new(Key::End)));
        assert_eq!(seen.get(), Some(8.0));
        assert!(scene.key_event(&KeyEvent::new(Key::PageDown)));
        assert_eq!(seen.get(), Some(0.0));
        assert!(!scene.key_event(&KeyEvent::new(Key::Enter)));
    }

    #[test]
    fn the_time_selector_changes_one_field_and_keeps_the_other() {
        let mut registry = Registry::new();
        registry.register(TimeSelectorComponent);
        let seen = Rc::new(Cell::new(None));
        let sink = seen.clone();
        let mut given = colors();
        given.push(("minutes".to_owned(), Prop::number(8.0 * 60.0 + 30.0)));
        given.push(("minute_step".to_owned(), Prop::number(5.0)));
        given.push(("font_family".to_owned(), Prop::text("Adwaita Mono")));
        given.push(("on_change".to_owned(), Prop::NumberEvent(Rc::new(move |v| sink.set(Some(v))))));
        let mut scene = Scene::new(Fonts::new());
        let root = scene.root();
        let row = registry.build("TimeSelector", &mut scene, root, given).unwrap();
        scene.set_viewport(Vec2::new(400.0, 100.0), 1.0);
        scene.frame();
        let fields = scene.get(row).unwrap().children().to_vec();
        let plus_of = |scene: &Scene, field: NodeId| {
            let plus = scene.get(field).unwrap().children()[2];
            let g = scene.geometry(plus).unwrap();
            Vec2::new(g.x + 5.0, g.y + 5.0)
        };
        let hour_plus = plus_of(&scene, fields[0]);
        click(&mut scene, hour_plus);
        assert_eq!(seen.get(), Some(9.0 * 60.0 + 30.0));
        let minute_plus = plus_of(&scene, fields[2]);
        click(&mut scene, minute_plus);
        assert_eq!(seen.get(), Some(8.0 * 60.0 + 35.0));
    }
}
