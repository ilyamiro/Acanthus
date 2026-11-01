//! Checkbox and radio group: a labelled indicator that eases between unselected and selected.

use std::rc::Rc;

use acanthus_anim::{Easing, Keep};
use acanthus_core::{Color, Signal, Vec2};
use acanthus_scene::{Anchors, CursorShape, Key, MouseArea, MouseButton, Node, NodeId, Path, Scene};

use crate::component::{Component, Prop, PropSpec, Props};
use crate::timing::{BASE, FAST};

/// Colours, font and metrics shared by [`checkbox`] and [`radio_group`].
#[derive(Clone)]
pub struct ChoiceStyle {
    /// Fill of a selected checkbox, ring and dot of a selected radio.
    pub accent: Signal<Color>,
    /// Outline of an unselected indicator.
    pub border: Signal<Color>,
    /// Tick colour of a checkbox.
    pub mark: Signal<Color>,
    /// Label colour.
    pub text: Signal<Color>,
    /// Label font family.
    pub font_family: Signal<String>,
    /// Label font size.
    pub font_size: f32,
    /// Width and height of the indicator.
    pub size: f32,
    /// Space between the indicator and the label.
    pub gap: f32,
}

#[derive(Clone, Copy, PartialEq)]
enum Shape {
    Box,
    Round,
}

/// Index of the option a key press moves the selection to, or `None` when the key does not move it.
pub fn step_selection(key: Key, current: usize, count: usize) -> Option<usize> {
    if count == 0 {
        return None;
    }
    match key {
        Key::Up | Key::Left => Some(current.saturating_sub(1)),
        Key::Down | Key::Right => Some((current + 1).min(count - 1)),
        _ => None,
    }
}

struct Row<'a> {
    style: &'a ChoiceStyle,
    shape: Shape,
    selected: Signal<bool>,
    label: &'a Signal<String>,
    enabled: &'a Signal<bool>,
    activate: Rc<dyn Fn()>,
    arrow: Option<Rc<dyn Fn(Key) -> bool>>,
}

fn option(scene: &mut Scene, parent: NodeId, row: Row<'_>) -> NodeId {
    let Row { style, shape, selected, label, enabled, activate, arrow } = row;
    let mut keep = Keep::default();
    let size = style.size;
    let area = {
        let (activate, enabled) = (activate.clone(), enabled.clone());
        MouseArea::new().on_click(move |button| {
            if button == MouseButton::Left && enabled.get_untracked() {
                activate();
            }
        })
    };
    let ring = {
        let (selected, accent, border) = (selected.clone(), style.accent.clone(), style.border.clone());
        let target = keep.derive(move || if selected.get() { accent.get() } else { border.get() });
        keep.ease(&target, FAST, Easing::Linear)
    };
    let fill = {
        let (selected, accent) = (selected.clone(), style.accent.clone());
        let target = keep.derive(move || match (shape, selected.get()) {
            (Shape::Box, true) => accent.get(),
            _ => Color::TRANSPARENT,
        });
        keep.ease(&target, FAST, Easing::Linear)
    };
    let shown = {
        let selected = selected.clone();
        let target = keep.derive(move || if selected.get() { 1.0_f32 } else { 0.0 });
        keep.ease(&target, BASE, Easing::OutBack(1.2))
    };
    let dim = {
        let enabled = enabled.clone();
        keep.derive(move || if enabled.get() { 1.0_f32 } else { 0.5 })
    };
    let text = Node::text(label).font_family(&style.font_family).font_size(style.font_size).color(&style.text);
    let label_width = text.implicit_size();
    let width = {
        let gap = style.gap;
        keep.derive(move || size + gap + label_width.get().x)
    };
    let mut root =
        Node::item().width(&width).height(size).opacity(&dim).mouse(area).cursor(CursorShape::Pointer).focusable(true);
    root = root.on_key(move |event| {
        if matches!(event.key, Key::Char(' ') | Key::Enter) {
            activate();
            true
        } else {
            arrow.as_ref().is_some_and(|arrow| arrow(event.key))
        }
    });
    let root = scene.add(parent, root);
    let radius = if shape == Shape::Box { size * 0.28 } else { size / 2.0 };
    let indicator =
        scene.add(root, Node::rect().size(size, size).radius(radius).color(&fill).border(size * 0.1, &ring));
    let mark = style.mark.clone();
    match shape {
        Shape::Box => {
            scene.add(
                indicator,
                Node::canvas(move |painting| {
                    let at = |x: f32, y: f32| Vec2::new(x * size, y * size);
                    let tick = Path::new().move_to(at(0.25, 0.52)).line_to(at(0.43, 0.7)).line_to(at(0.75, 0.3));
                    painting.stroke(&tick, mark.get(), size * 0.12);
                })
                .size(size, size)
                .opacity(&shown),
            );
        }
        Shape::Round => {
            let dot = size * 0.5;
            scene.add(
                indicator,
                Node::rect()
                    .size(dot, dot)
                    .radius(dot / 2.0)
                    .color(&style.accent)
                    .anchors(Anchors::center())
                    .scale(&shown),
            );
        }
    }
    scene.add(root, text.anchors(Anchors::default().left(size + style.gap).v_center()));
    if let Some(node) = scene.get_mut(root) {
        node.keep(keep);
    }
    root
}

/// Description of a checkbox.
pub struct Checkbox {
    /// Colours and metrics.
    pub style: ChoiceStyle,
    /// Label next to the box.
    pub label: Signal<String>,
    /// Whether the box is ticked; the caller flips it from `on_toggle`.
    pub checked: Signal<bool>,
    /// Ignores input and dims while false.
    pub enabled: Signal<bool>,
    /// Called with the requested state on a click, Space or Enter.
    pub on_toggle: Rc<dyn Fn(bool)>,
}

/// Builds a checkbox under `parent` and returns its row.
pub fn checkbox(scene: &mut Scene, parent: NodeId, spec: Checkbox) -> NodeId {
    let Checkbox { style, label, checked, enabled, on_toggle } = spec;
    let activate = {
        let (checked, enabled) = (checked.clone(), enabled.clone());
        Rc::new(move || {
            if enabled.get_untracked() {
                on_toggle(!checked.get_untracked());
            }
        })
    };
    option(
        scene,
        parent,
        Row {
            style: &style,
            shape: Shape::Box,
            selected: checked,
            label: &label,
            enabled: &enabled,
            activate,
            arrow: None,
        },
    )
}

/// Description of a radio group.
pub struct RadioGroup {
    /// Colours and metrics.
    pub style: ChoiceStyle,
    /// One label per option.
    pub options: Vec<Signal<String>>,
    /// Index of the selected option; the caller updates it from `on_select`.
    pub current: Signal<usize>,
    /// Ignores input and dims while false.
    pub enabled: Signal<bool>,
    /// Vertical space between options.
    pub spacing: f32,
    /// Called with the index picked by a click, Space, Enter or an arrow key.
    pub on_select: Rc<dyn Fn(usize)>,
}

/// Builds a column of radio options under `parent` and returns it.
pub fn radio_group(scene: &mut Scene, parent: NodeId, spec: RadioGroup) -> NodeId {
    let RadioGroup { style, options, current, enabled, spacing, on_select } = spec;
    let column = scene.add(parent, Node::column(spacing));
    let count = options.len();
    let mut keep = Keep::default();
    for (index, label) in options.iter().enumerate() {
        let selected = {
            let current = current.clone();
            keep.derive(move || current.get() == index)
        };
        let activate: Rc<dyn Fn()> = {
            let (enabled, on_select) = (enabled.clone(), on_select.clone());
            Rc::new(move || {
                if enabled.get_untracked() {
                    on_select(index);
                }
            })
        };
        let arrow: Rc<dyn Fn(Key) -> bool> = {
            let (enabled, on_select) = (enabled.clone(), on_select.clone());
            Rc::new(move |key| match step_selection(key, index, count) {
                Some(next) if enabled.get_untracked() => {
                    on_select(next);
                    true
                }
                _ => false,
            })
        };
        option(
            scene,
            column,
            Row {
                style: &style,
                shape: Shape::Round,
                selected,
                label,
                enabled: &enabled,
                activate,
                arrow: Some(arrow),
            },
        );
    }
    if let Some(node) = scene.get_mut(column) {
        node.keep(keep);
    }
    column
}

fn style_from(props: &Props) -> ChoiceStyle {
    ChoiceStyle {
        accent: props.color("accent"),
        border: props.color("border"),
        mark: props.color("mark"),
        text: props.color("text"),
        font_family: props.text("font_family"),
        font_size: props.number("font_size").get_untracked(),
        size: props.number("size").get_untracked(),
        gap: props.number("gap").get_untracked(),
    }
}

macro_rules! with_style_props {
    ($($extra:expr),* $(,)?) => {
        &[
            PropSpec::text("font_family", || Prop::text("")),
            PropSpec::number("font_size", || Prop::number(14.0)),
            PropSpec::number("size", || Prop::number(20.0)),
            PropSpec::scaled("gap", "spacing.sm", || Prop::number(8.0)),
            PropSpec::themed("accent", "primary"),
            PropSpec::themed("border", "surface2"),
            PropSpec::themed("mark", "base"),
            PropSpec::themed("text", "text"),
            $($extra),*
        ]
    };
}

/// The checkbox as a [`Component`] named `Checkbox`.
pub struct CheckboxComponent;

impl Component for CheckboxComponent {
    fn name(&self) -> &'static str {
        "Checkbox"
    }

    fn props(&self) -> &'static [PropSpec] {
        const PROPS: &[PropSpec] = with_style_props![
            PropSpec::text("label", || Prop::text("")),
            PropSpec::bool("checked", || Prop::bool(false)),
            PropSpec::bool("enabled", || Prop::bool(true)),
            PropSpec::bool_event("on_toggle"),
        ];
        PROPS
    }

    fn build(&self, scene: &mut Scene, parent: NodeId, props: &Props) -> NodeId {
        checkbox(
            scene,
            parent,
            Checkbox {
                style: style_from(props),
                label: props.text("label"),
                checked: props.bool("checked"),
                enabled: props.bool("enabled"),
                on_toggle: props.bool_event("on_toggle"),
            },
        )
    }
}

/// The radio group as a [`Component`] named `RadioGroup`.
pub struct RadioGroupComponent;

impl Component for RadioGroupComponent {
    fn name(&self) -> &'static str {
        "RadioGroup"
    }

    fn props(&self) -> &'static [PropSpec] {
        const PROPS: &[PropSpec] = with_style_props![
            PropSpec::text_slots("options"),
            PropSpec::index("current", || Prop::Index(Signal::new(0))),
            PropSpec::bool("enabled", || Prop::bool(true)),
            PropSpec::scaled("spacing", "spacing.sm", || Prop::number(8.0)),
            PropSpec::index_event("on_select"),
        ];
        PROPS
    }

    fn build(&self, scene: &mut Scene, parent: NodeId, props: &Props) -> NodeId {
        radio_group(
            scene,
            parent,
            RadioGroup {
                style: style_from(props),
                options: props.text_slots("options"),
                current: props.index("current"),
                enabled: props.bool("enabled"),
                spacing: props.number("spacing").get_untracked(),
                on_select: props.index_event("on_select"),
            },
        )
    }
}

#[cfg(test)]
mod tests {
    use std::cell::{Cell, RefCell};

    use acanthus_scene::{Fonts, KeyEvent};

    use super::*;
    use crate::component::Registry;

    fn colors() -> Vec<(String, Prop)> {
        ["accent", "border", "mark", "text"].map(|name| (name.to_owned(), Prop::color(Color::hex(0x336699)))).to_vec()
    }

    fn click(scene: &mut Scene, at: Vec2) {
        scene.pointer_motion_at(at, 0.0);
        scene.pointer_button_at(MouseButton::Left, true, 0.0);
        scene.pointer_button_at(MouseButton::Left, false, 0.0);
    }

    #[test]
    fn arrows_clamp_at_both_ends() {
        assert_eq!(step_selection(Key::Down, 0, 3), Some(1));
        assert_eq!(step_selection(Key::Down, 2, 3), Some(2));
        assert_eq!(step_selection(Key::Up, 0, 3), Some(0));
        assert_eq!(step_selection(Key::Left, 2, 3), Some(1));
        assert_eq!(step_selection(Key::Enter, 1, 3), None);
        assert_eq!(step_selection(Key::Down, 0, 0), None);
    }

    #[test]
    fn a_checkbox_asks_for_the_flipped_state_unless_disabled() {
        let mut registry = Registry::new();
        registry.register(CheckboxComponent);
        let seen = Rc::new(Cell::new(None));
        let sink = seen.clone();
        let checked = Signal::new(false);
        let enabled = Signal::new(true);
        let mut given = colors();
        given.push(("label".to_owned(), Prop::text("Remember me")));
        given.push(("checked".to_owned(), Prop::Bool(checked)));
        given.push(("enabled".to_owned(), Prop::Bool(enabled.clone())));
        given.push(("on_toggle".to_owned(), Prop::BoolEvent(Rc::new(move |v| sink.set(Some(v))))));
        let mut scene = Scene::new(Fonts::new());
        let root = scene.root();
        let node = registry.build("Checkbox", &mut scene, root, given).unwrap();
        scene.set_viewport(Vec2::new(300.0, 100.0), 1.0);
        scene.frame();
        assert!(scene.geometry(node).is_some_and(|g| g.h == 20.0 && g.w >= 28.0));
        click(&mut scene, Vec2::new(5.0, 5.0));
        assert_eq!(seen.get(), Some(true));
        seen.set(None);
        enabled.set(false);
        click(&mut scene, Vec2::new(5.0, 5.0));
        assert_eq!(seen.get(), None);
    }

    #[test]
    fn a_radio_group_selects_by_click_and_arrow_key() {
        let mut registry = Registry::new();
        registry.register(RadioGroupComponent);
        let picked = Rc::new(RefCell::new(Vec::new()));
        let sink = picked.clone();
        let mut given = colors();
        let labels = ["One", "Two", "Three"].map(|t| Signal::new(t.to_owned()));
        given.push(("options".to_owned(), Prop::TextSlots(labels.to_vec())));
        given.push(("current".to_owned(), Prop::Index(Signal::new(0))));
        given.push(("on_select".to_owned(), Prop::IndexEvent(Rc::new(move |i| sink.borrow_mut().push(i)))));
        let mut scene = Scene::new(Fonts::new());
        let root = scene.root();
        let group = registry.build("RadioGroup", &mut scene, root, given).unwrap();
        scene.set_viewport(Vec2::new(300.0, 200.0), 1.0);
        scene.frame();
        let rows = scene.get(group).unwrap().children().to_vec();
        assert_eq!(rows.len(), 3);
        let second = scene.geometry(rows[1]).unwrap();
        click(&mut scene, Vec2::new(second.x + 5.0, second.y + 5.0));
        assert_eq!(*picked.borrow(), vec![1]);
        scene.set_focus(Some(rows[1]));
        assert!(scene.key_event(&KeyEvent::new(Key::Down)));
        assert_eq!(*picked.borrow(), vec![1, 2]);
    }
}
