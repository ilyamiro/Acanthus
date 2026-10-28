//! Dropdown selector: a button that shows the current option and opens a floating list with a
//! staggered entrance. Double-clicking the button switches it to a search field that filters the
//! options by fuzzy score; Enter picks the best match, Escape or a click outside closes the list.

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::{Duration, Instant};

use acanthus_anim::{AnimHandle, Easing, Keep, Tween, start};
use acanthus_core::{Color, Effect, ListSignal, Signal, untrack};
use acanthus_scene::{
    Anchors, CursorShape, Elide, Item, Key, ListViewConfig, MouseArea, MouseButton, Node, NodeId, Scene, TextInput,
    TextInputStyle, TransformOrigin,
};

use crate::button::{Feel, Press, Sound, activate, dress, focusable};
use crate::component::{Component, Kind, Prop, PropSpec, Props};
use crate::scrollbar::{ScrollBar, scroll_bar};
use crate::timing::{BASE, FAST, SLOW};

/// Height of one list row, in unscaled logical pixels.
pub const ITEM_HEIGHT: f32 = 32.0;
/// Vertical gap between rows, in unscaled logical pixels.
pub const ITEM_GAP: f32 = 2.0;
/// Height above which the list scrolls instead of growing, in unscaled logical pixels.
pub const MAX_LIST_HEIGHT: f32 = 250.0;
/// Padding between the menu frame and the list, in unscaled logical pixels.
pub const PADDING: f32 = 4.0;
/// Gap between the button and the menu below it, in unscaled logical pixels.
pub const MENU_GAP: f32 = 6.0;
/// Duration of the menu's opening animation, in milliseconds (OutQuint).
pub const ENTER_MS: u32 = 250;
/// Duration of the menu's closing animation, in milliseconds (InQuint).
pub const EXIT_MS: u32 = 150;
/// Relative path of the sound played when the button or a row is clicked.
pub const CLICK_SOUND: &str = "reusables/dropdown/click.wav";
const DOUBLE_CLICK: Duration = Duration::from_millis(400);

/// Scores how well `query` matches `text` as an in-order, case-insensitive subsequence.
///
/// Returns `None` when some query character is missing. Every matched character earns 10, runs of
/// consecutive matches earn a growing bonus, and matches at the start of a word (after `/`, `_`,
/// `-`, space or `.`) earn 15 more. An empty query matches everything with score 1.
pub fn fuzzy_score(query: &str, text: &str) -> Option<i32> {
    let query: Vec<char> = query.chars().flat_map(char::to_lowercase).collect();
    if query.is_empty() {
        return Some(1);
    }
    let text: Vec<char> = text.chars().flat_map(char::to_lowercase).collect();
    if query.len() > text.len() {
        return None;
    }
    let (mut matched, mut score, mut consecutive, mut previous) = (0, 0i32, 0i32, None::<usize>);
    for (at, ch) in text.iter().enumerate() {
        if matched == query.len() {
            break;
        }
        if *ch != query[matched] {
            continue;
        }
        score += 10;
        if previous.is_some_and(|p| p + 1 == at) {
            consecutive += 1;
            score += consecutive * 5;
        } else {
            consecutive = 0;
        }
        if at == 0 || matches!(text[at - 1], '/' | '_' | '-' | ' ' | '.') {
            score += 15;
        }
        previous = Some(at);
        matched += 1;
    }
    (matched == query.len()).then_some(score)
}

/// Returns the indices of the options matching `query`, best score first and in original order
/// among equal scores. An empty query keeps every option in order.
pub fn filter(options: &[String], query: &str) -> Vec<usize> {
    if query.is_empty() {
        return (0..options.len()).collect();
    }
    let mut hits: Vec<(i32, usize)> =
        options.iter().enumerate().filter_map(|(i, o)| fuzzy_score(query, o).map(|score| (score, i))).collect();
    hits.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
    hits.into_iter().map(|(_, i)| i).collect()
}

const FEEL: Feel =
    Feel { pop_peak: 1.04, pop_in_ms: 100, pop_out_ms: SLOW, flash_peak: 0.2, flash_ms: SLOW, sound: CLICK_SOUND };

/// Unscaled height of `count` rows including the gaps between them.
pub fn content_height(count: usize) -> f32 {
    if count == 0 { 0.0 } else { count as f32 * ITEM_HEIGHT + (count - 1) as f32 * ITEM_GAP }
}

/// Unscaled height of the list viewport for `count` rows, capped at [`MAX_LIST_HEIGHT`].
pub fn list_height(count: usize) -> f32 {
    content_height(count).min(MAX_LIST_HEIGHT)
}

/// Number of rows that fit in the viewport without scrolling (the rows that take part in the stagger).
pub fn visible_count(count: usize) -> usize {
    count.min(((MAX_LIST_HEIGHT + 2.0) / (ITEM_HEIGHT + ITEM_GAP)) as usize)
}

/// Delay before row `index` appears after the menu starts opening, in milliseconds.
pub fn stagger_ms(index: usize) -> f32 {
    index as f32 * 40.0 + 20.0
}

/// Time until the last staggered row of a `count`-row list has appeared, in milliseconds.
pub fn stagger_total_ms(count: usize) -> f32 {
    stagger_ms(visible_count(count).saturating_sub(1)) + 1.0
}

/// Whether row `index` of `count` rows is visible `elapsed_ms` after the menu started opening;
/// rows below the viewport show at once.
pub fn shown(elapsed_ms: f32, index: usize, count: usize) -> bool {
    index >= visible_count(count) || elapsed_ms >= stagger_ms(index)
}

/// Corner radius of the menu frame for a button with corner radius `radius`.
pub fn menu_radius(radius: f32) -> f32 {
    radius.min(16.0)
}

/// Corner radius of a row inside a menu for a button with corner radius `radius`.
pub fn row_radius(radius: f32) -> f32 {
    (radius - 4.0).clamp(2.0, 16.0)
}

/// Description of a dropdown: options, selection, placement, metrics, colours and handlers.
pub struct Spec {
    /// Option labels shown in the list.
    pub options: Signal<Vec<String>>,
    /// Index of the selected option in [`Spec::options`].
    pub current: Signal<usize>,
    /// Text shown when nothing valid is selected, and the hint of the search field.
    pub placeholder: Signal<String>,
    /// Whether the menu is open; the widget writes it and the caller may set it too.
    pub open: Signal<bool>,
    /// Node that hosts the menu and the click-outside catcher, usually a full-size top layer so the
    /// menu is drawn above siblings and is not clipped.
    pub overlay: NodeId,
    /// Size of [`Spec::overlay`]; the click-outside catcher covers it.
    pub overlay_size: (f32, f32),
    /// Horizontal position of the button inside its parent.
    pub x: Signal<f32>,
    /// Vertical position of the button inside its parent.
    pub y: Signal<f32>,
    /// Horizontal position of the menu inside [`Spec::overlay`].
    pub menu_x: Signal<f32>,
    /// Vertical position of the button inside [`Spec::overlay`]; the menu opens below it.
    pub menu_y: Signal<f32>,
    /// Width of the button and the menu.
    pub width: f32,
    /// Height of the button.
    pub height: f32,
    /// Scale factor applied to paddings, rows and gaps.
    pub scale: f32,
    /// Corner radius of the button; the menu and rows derive theirs from it.
    pub radius: Signal<f32>,
    /// Font family of the labels.
    pub font: Signal<String>,
    /// Font size of the labels.
    pub font_size: f32,
    /// Size of the chevron icon.
    pub icon_size: f32,
    /// Colour of the selected row, the caret and the search selection.
    pub accent: Signal<Color>,
    /// Button fill at rest.
    pub base: Signal<Color>,
    /// Button fill while hovered or open, and the fill of hovered rows.
    pub hover: Signal<Color>,
    /// Fill of the menu frame.
    pub menu: Signal<Color>,
    /// Colour of the menu border.
    pub border: Signal<Color>,
    /// Colour of labels and the chevron.
    pub text: Signal<Color>,
    /// Label colour of the selected row.
    pub active_text: Signal<Color>,
    /// Sound player for clicks.
    pub sound: Sound,
    /// Called with the index in [`Spec::options`] of the option the user picked.
    pub on_select: Rc<dyn Fn(usize)>,
}

#[derive(Default)]
struct Run(RefCell<Option<AnimHandle>>);

impl Run {
    fn set(&self, handle: AnimHandle) {
        if let Some(old) = self.0.borrow_mut().replace(handle) {
            old.stop();
        }
    }

    fn stop(&self) {
        if let Some(handle) = self.0.borrow_mut().take() {
            handle.stop();
        }
    }
}

impl Drop for Run {
    fn drop(&mut self) {
        self.stop();
    }
}

/// Builds a dropdown under `parent` (its menu under [`Spec::overlay`]) and returns the button node.
pub fn dropdown(scene: &mut Scene, parent: NodeId, spec: Spec) -> NodeId {
    let mut keep = Keep::default();
    let s = spec.scale;
    let open = spec.open.clone();
    let presence = Signal::new(0.0f32);
    let elapsed = Signal::new(0.0f32);
    let editing = Signal::new(false);
    let input = {
        let (open, editing) = (open.clone(), editing.clone());
        TextInput::new("")
            .on_accepted({
                let (open, editing) = (open.clone(), editing.clone());
                let (options, on_select) = (spec.options.clone(), spec.on_select.clone());
                move |text| {
                    let first = options.with(|list| filter(list, text).first().copied());
                    if let Some(index) = first {
                        on_select(index);
                    }
                    open.set(false);
                    editing.set(false);
                }
            })
            .before_key(move |event| {
                if event.key == Key::Escape && editing.get_untracked() {
                    open.set(false);
                    editing.set(false);
                    true
                } else {
                    false
                }
            })
    };
    let matches = {
        let (options, query) = (spec.options.clone(), input.text());
        keep.derive(move || options.with(|list| query.with(|q| filter(list, q))))
    };
    let count = {
        let matches = matches.clone();
        keep.derive(move || matches.with(Vec::len))
    };
    {
        let (editing, open, matches, input) = (editing.clone(), open.clone(), matches.clone(), input.clone());
        keep.add(Effect::new(move || {
            let on = editing.get();
            let empty = matches.with(Vec::is_empty);
            untrack(|| {
                if on {
                    open.set(!empty);
                } else {
                    input.clear();
                }
            });
        }));
    }
    let presence_run = Rc::new(Run::default());
    let elapsed_run = Rc::new(Run::default());
    {
        let (open, presence, elapsed, count) = (open.clone(), presence.clone(), elapsed.clone(), count.clone());
        let (presence_run, elapsed_run) = (presence_run.clone(), elapsed_run.clone());
        keep.add(Effect::new(move || {
            let is_open = open.get();
            untrack(|| {
                if is_open {
                    presence_run.set(start(Tween::new(&presence, 1.0, ENTER_MS, Easing::OutQuint)));
                    elapsed.set(0.0);
                    let total = stagger_total_ms(count.get_untracked());
                    elapsed_run.set(start(Tween::new(&elapsed, total, total as u32, Easing::Linear)));
                } else {
                    presence_run.set(start(Tween::new(&presence, 0.0, EXIT_MS, Easing::InQuint)));
                    elapsed_run.stop();
                    elapsed.set(0.0);
                }
            });
        }));
    }
    keep.add(presence_run);
    keep.add(elapsed_run);

    let area = MouseArea::new();
    let (hovered, pressed) = (area.hovered(), area.pressed());
    let press = Press::new(FEEL, spec.sound.clone());
    let toggle = {
        let (open, count, editing) = (open.clone(), count.clone(), editing.clone());
        let last_click = Cell::new(None::<Instant>);
        activate(&press, move || {
            let now = Instant::now();
            let double = last_click.replace(Some(now)).is_some_and(|t| now.duration_since(t) <= DOUBLE_CLICK);
            if editing.get_untracked() {
                editing.set(false);
                open.set(false);
            } else if double {
                last_click.set(None);
                editing.set(true);
            } else if open.get_untracked() {
                open.set(false);
            } else if count.get_untracked() > 0 {
                open.set(true);
            }
        })
    };
    let area = {
        let toggle = toggle.clone();
        area.on_click(move |button| {
            if button == MouseButton::Left {
                toggle();
            }
        })
    };
    let scale_target = {
        let pressed = pressed.clone();
        keep.derive(move || if pressed.get() { 0.96f32 } else { 1.0 })
    };
    let body_scale = press.scale(&mut keep, &scale_target);
    let fill_target = {
        let (open, hovered, base, hover) = (open.clone(), hovered, spec.base.clone(), spec.hover.clone());
        keep.derive(move || if open.get() || hovered.get() { hover.get() } else { base.get() })
    };
    let fill = keep.ease(&fill_target, FAST, Easing::Linear);
    let rotation_target = {
        let open = open.clone();
        keep.derive(move || if open.get() { 90.0f32 } else { -90.0 })
    };
    let rotation = keep.ease(&rotation_target, BASE, Easing::OUT_BACK);
    let label = {
        let (options, current, placeholder) = (spec.options.clone(), spec.current.clone(), spec.placeholder.clone());
        keep.derive(move || {
            let value = options.with(|list| list.get(current.get()).cloned().unwrap_or_default());
            if value.is_empty() { placeholder.get() } else { value }
        })
    };
    keep.add(press.clone());
    let (menu_x, menu_y) = {
        let (x, y) = (spec.menu_x.clone(), spec.menu_y.clone());
        let height = spec.height;
        (x, keep.derive(move || y.get() + height + MENU_GAP * s))
    };
    let radius = spec.radius.clone();

    let outer = focusable(
        Node::item()
            .x(&spec.x)
            .y(&spec.y)
            .size(spec.width, spec.height)
            .mouse(area)
            .cursor(CursorShape::Pointer)
            .on_key({
                let open = open.clone();
                move |event| {
                    if event.key == Key::Escape && open.get_untracked() {
                        open.set(false);
                        true
                    } else {
                        false
                    }
                }
            }),
        {
            let (open, toggle) = (open.clone(), toggle);
            Rc::new(move || {
                if !open.get_untracked() {
                    toggle();
                }
            })
        },
    );
    let outer = scene.add(parent, outer);
    let body = scene.add(outer, Node::item().anchors(Anchors::fill(0.0)).scale(&body_scale));
    scene.add(body, Node::rect().anchors(Anchors::fill(0.0)).radius(&radius).color(&fill));
    let pad = (12.0 * s).round();
    let chevron_room = pad + spec.icon_size + (8.0 * s).round();
    let browsing = {
        let editing = editing.clone();
        keep.derive(move || !editing.get())
    };
    scene.add(
        body,
        Node::text(&label)
            .x(pad)
            .width(spec.width - pad - chevron_room)
            .height(spec.height)
            .visible(&browsing)
            .font_family(&spec.font)
            .font_size(spec.font_size)
            .color(&spec.text)
            .elide(Elide::Right)
            .max_lines(1)
            .v_align(acanthus_scene::VAlign::Center),
    );
    search_input(scene, body, &spec, &input, &editing, pad, spec.width - pad - chevron_room);
    chevron(scene, body, &spec, &rotation, spec.width - pad - spec.icon_size);
    dress(scene, body, &radius, &press);

    let catcher = {
        let open = open.clone();
        Node::item().x(0.0).y(0.0).size(spec.overlay_size.0, spec.overlay_size.1).z(99.0).visible(&open).mouse(
            MouseArea::new().on_click({
                let open = open.clone();
                move |_| open.set(false)
            }),
        )
    };
    scene.add(spec.overlay, catcher);
    menu(scene, &spec, &matches, &presence, &elapsed, &menu_x, &menu_y);
    if let Some(node) = scene.get_mut(outer) {
        node.keep(keep);
    }
    outer
}

fn search_input(
    scene: &mut Scene,
    body: NodeId,
    spec: &Spec,
    input: &TextInput,
    editing: &Signal<bool>,
    x: f32,
    width: f32,
) {
    let mut keep = Keep::default();
    let field_height = (spec.font_size * 1.7).round();
    let style = TextInputStyle {
        font_size: Signal::new(spec.font_size),
        font_family: spec.font.clone(),
        color: spec.text.clone(),
        caret_color: spec.accent.clone(),
        selection_color: keep.alpha(&spec.accent, 0.28),
        placeholder: spec.placeholder.clone(),
        placeholder_color: keep.alpha(&spec.text, 0.45),
        ..TextInputStyle::default()
    };
    let holder = scene.add(body, Node::item().x(x).size(width, spec.height).visible(editing).own(keep));
    let nodes = scene.text_input(
        holder,
        Node::item().y(((spec.height - field_height) / 2.0).round()).size(width, field_height),
        input,
        style,
    );
    scene.focus_when(nodes.field, editing.clone());
}

fn chevron(scene: &mut Scene, body: NodeId, spec: &Spec, rotation: &Signal<f32>, x: f32) {
    let size = spec.icon_size;
    let unit = size / 0.38;
    let thickness = (unit * 0.048).max(1.5);
    let arm = (unit * 0.19).max(5.0);
    let holder =
        scene.add(body, Node::item().x(x).y(((spec.height - size) / 2.0).round()).size(size, size).rotation(rotation));
    let pivot = scene.add(holder, Node::item().x((size - arm * 0.766) / 2.0).y(size / 2.0).size(0.0, 0.0));
    scene.add(
        pivot,
        Node::rect()
            .x(-thickness / 2.0)
            .y(-thickness / 2.0)
            .size(thickness, thickness)
            .radius(thickness / 2.0)
            .color(&spec.text),
    );
    for angle in [40.0f32, -40.0] {
        scene.add(
            pivot,
            Node::rect()
                .x(0.0)
                .y(-thickness / 2.0)
                .size(arm, thickness)
                .radius(thickness / 2.0)
                .transform_origin(TransformOrigin::Left)
                .rotation(angle)
                .color(&spec.text),
        );
    }
}

fn menu(
    scene: &mut Scene,
    spec: &Spec,
    matches: &Signal<Vec<usize>>,
    presence: &Signal<f32>,
    elapsed: &Signal<f32>,
    x: &Signal<f32>,
    y: &Signal<f32>,
) {
    let mut keep = Keep::default();
    let s = spec.scale;
    let pad = (PADDING * s).round();
    let count = {
        let matches = matches.clone();
        keep.derive(move || matches.with(Vec::len))
    };
    let visible = {
        let (open, presence) = (spec.open.clone(), presence.clone());
        keep.derive(move || open.get() || presence.get() > 0.001)
    };
    let list_h = {
        let count = count.clone();
        keep.derive(move || (list_height(count.get()) * s).round())
    };
    let menu_h = {
        let list_h = list_h.clone();
        keep.derive(move || list_h.get() + pad * 2.0)
    };
    let scale = {
        let presence = presence.clone();
        keep.derive(move || 0.9 + 0.1 * presence.get())
    };
    let menu_radius = {
        let radius = spec.radius.clone();
        keep.derive(move || menu_radius(radius.get()))
    };
    let wrapper = scene.add(
        spec.overlay,
        Node::item()
            .x(x)
            .y(y)
            .size(spec.width, &menu_h)
            .z(100.0)
            .visible(&visible)
            .opacity(presence)
            .scale(&scale)
            .transform_origin(TransformOrigin::Top)
            .mouse(MouseArea::new()),
    );
    scene.add(wrapper, Node::rect().anchors(Anchors::fill(0.0)).radius(&menu_radius).color(&spec.menu));

    let shown_rows = |options: &[String], matches: &[usize]| -> Vec<String> {
        matches.iter().filter_map(|&i| options.get(i).cloned()).collect()
    };
    let rows: ListSignal<String> = ListSignal::new(shown_rows(&spec.options.get_untracked(), &matches.get_untracked()));
    {
        let (options, matches, rows) = (spec.options.clone(), matches.clone(), rows.clone());
        keep.add(Signal::new(()).bind(move || {
            rows.replace(options.with(|list| matches.with(|m| shown_rows(list, m))));
        }));
    }
    let inner_w = spec.width - pad * 2.0;
    let row = Rc::new(Row {
        width: inner_w,
        scale: s,
        radius: {
            let radius = spec.radius.clone();
            keep.derive(move || row_radius(radius.get() / s.max(0.01)) * s)
        },
        font: spec.font.clone(),
        font_size: spec.font_size,
        accent: spec.accent.clone(),
        hover: spec.hover.clone(),
        text: spec.text.clone(),
        active_text: spec.active_text.clone(),
        current: spec.current.clone(),
        matches: matches.clone(),
        elapsed: elapsed.clone(),
        count: count.clone(),
        open: spec.open.clone(),
        sound: spec.sound.clone(),
        on_select: spec.on_select.clone(),
        overflow: Signal::new(false),
    });
    let view = scene.list_view(
        wrapper,
        Node::item().x(pad).y(pad).size(inner_w, &list_h).clip(true),
        ListViewConfig::default().spacing(ITEM_GAP * s),
        &rows,
        |value: &String| value.clone(),
        {
            let row = row.clone();
            move |scene, parent, item| row.build(scene, parent, item)
        },
    );
    {
        let (overflow, flick, size) = (row.overflow.clone(), view.flickable.clone(), view.view_size.clone());
        keep.add(Signal::new(()).bind(move || overflow.set(flick.content_height.get() > size.get().y + 1.0)));
    }
    scroll_bar(
        scene,
        wrapper,
        ScrollBar {
            flickable: view.flickable.clone(),
            view: view.view_size.clone(),
            x: inner_w - (6.0 * s).round(),
            top: pad,
            width: (4.0 * s).round(),
            radius: (2.0 * s).round(),
            color: &spec.border,
        },
    );
    if let Some(node) = scene.get_mut(wrapper) {
        node.keep(keep);
    }
}

struct Row {
    width: f32,
    scale: f32,
    radius: Signal<f32>,
    font: Signal<String>,
    font_size: f32,
    accent: Signal<Color>,
    hover: Signal<Color>,
    text: Signal<Color>,
    active_text: Signal<Color>,
    current: Signal<usize>,
    matches: Signal<Vec<usize>>,
    elapsed: Signal<f32>,
    count: Signal<usize>,
    open: Signal<bool>,
    sound: Sound,
    on_select: Rc<dyn Fn(usize)>,
    overflow: Signal<bool>,
}

impl Row {
    fn build(&self, scene: &mut Scene, parent: NodeId, item: Item<String>) -> NodeId {
        let mut keep = Keep::default();
        let s = self.scale;
        let area = MouseArea::new();
        let (hovered, pressed) = (area.hovered(), area.pressed());
        let index = item.index.clone();
        let area = {
            let (open, on_select, sound, index, matches) =
                (self.open.clone(), self.on_select.clone(), self.sound.clone(), index.clone(), self.matches.clone());
            area.on_click(move |button| {
                if button != MouseButton::Left {
                    return;
                }
                if let Some(origin) = matches.with_untracked(|m| m.get(index.get_untracked()).copied()) {
                    on_select(origin);
                }
                open.set(false);
                if let Some(sound) = &sound {
                    sound(CLICK_SOUND);
                }
            })
        };
        let selected = {
            let (current, index, matches) = (self.current.clone(), index.clone(), self.matches.clone());
            keep.derive(move || matches.with(|m| m.get(index.get()) == Some(&current.get())))
        };
        let shown = {
            let (elapsed, index, count, open) =
                (self.elapsed.clone(), index.clone(), self.count.clone(), self.open.clone());
            keep.derive(move || {
                let present = if open.get() { 1.0f32 } else { 0.0 };
                if present > 0.0 && shown(elapsed.get(), index.get(), count.get()) { 1.0f32 } else { 0.0 }
            })
        };
        let appear = keep.ease(&shown, BASE, Easing::OutQuint);
        let lift = {
            let appear = appear.clone();
            keep.derive(move || (8.0 * s) * (1.0 - appear.get()))
        };
        let fill_target = {
            let (hover, accent, selected, hovered, pressed) =
                (self.hover.clone(), self.accent.clone(), selected.clone(), hovered.clone(), pressed.clone());
            keep.derive(move || {
                if selected.get() {
                    accent.get()
                } else if pressed.get() {
                    let base = hover.get();
                    Color { r: base.r / 1.1, g: base.g / 1.1, b: base.b / 1.1, a: base.a }
                } else if hovered.get() {
                    hover.get()
                } else {
                    Color::TRANSPARENT
                }
            })
        };
        let fill = keep.ease(&fill_target, FAST, Easing::Linear);
        let scale_target = {
            let (hovered, pressed) = (hovered, pressed);
            keep.derive(move || {
                if pressed.get() {
                    0.96f32
                } else if hovered.get() {
                    1.02
                } else {
                    1.0
                }
            })
        };
        let scale = keep.ease(&scale_target, BASE, Easing::OutQuint);
        let text_color_target = {
            let (selected, text, active) = (selected, self.text.clone(), self.active_text.clone());
            keep.derive(move || if selected.get() { active.get() } else { text.get() })
        };
        let text_color = keep.ease(&text_color_target, FAST, Easing::Linear);
        let right = {
            let overflow = self.overflow.clone();
            keep.derive(move || if overflow.get() { 12.0 * s } else { 8.0 * s })
        };
        let text_width = {
            let width = self.width;
            keep.derive(move || width - 8.0 * s - right.get())
        };
        let holder = scene.add(parent, Node::item().size(self.width, (ITEM_HEIGHT * s).round()).own(keep));
        let shifted = scene.add(
            holder,
            Node::item()
                .y(&lift)
                .size(self.width, (ITEM_HEIGHT * s).round())
                .opacity(&appear)
                .scale(&scale)
                .mouse(area)
                .cursor(CursorShape::Pointer),
        );
        scene.add(shifted, Node::rect().anchors(Anchors::fill(0.0)).radius(&self.radius).color(&fill));
        scene.add(
            shifted,
            Node::text(&item.value)
                .x((8.0 * s).round())
                .width(&text_width)
                .height((ITEM_HEIGHT * s).round())
                .font_family(&self.font)
                .font_size(self.font_size)
                .color(&text_color)
                .elide(Elide::Right)
                .max_lines(1)
                .v_align(acanthus_scene::VAlign::Center),
        );
        holder
    }
}

/// The dropdown as a [`Component`] named `Dropdown`; clicks make no sound.
pub struct DropdownComponent;

impl Component for DropdownComponent {
    fn name(&self) -> &'static str {
        "Dropdown"
    }

    fn props(&self) -> &'static [PropSpec] {
        const PROPS: &[PropSpec] = &[
            PropSpec::number("x", || Prop::number(0.0)),
            PropSpec::number("y", || Prop::number(0.0)),
            PropSpec::number("menu_x", || Prop::number(0.0)),
            PropSpec::number("menu_y", || Prop::number(0.0)),
            PropSpec::number("width", || Prop::number(200.0)),
            PropSpec::number("height", || Prop::number(36.0)),
            PropSpec::number("scale", || Prop::number(1.0)),
            PropSpec::number("radius", || Prop::number(10.0)),
            PropSpec::number("font_size", || Prop::number(14.0)),
            PropSpec::number("icon_size", || Prop::number(16.0)),
            PropSpec::number("overlay_width", || Prop::number(0.0)),
            PropSpec::number("overlay_height", || Prop::number(0.0)),
            PropSpec::text("font", || Prop::text("")),
            PropSpec::text("placeholder", || Prop::text("")),
            PropSpec::bool("open", || Prop::bool(false)),
            PropSpec::index("current", || Prop::Index(Signal::new(0))),
            PropSpec::text_list("options"),
            PropSpec::required("overlay", Kind::Node),
            PropSpec::required("accent", Kind::Color),
            PropSpec::required("base", Kind::Color),
            PropSpec::required("hover", Kind::Color),
            PropSpec::required("menu", Kind::Color),
            PropSpec::required("border", Kind::Color),
            PropSpec::required("text", Kind::Color),
            PropSpec::required("active_text", Kind::Color),
            PropSpec::index_event("on_select"),
        ];
        PROPS
    }

    fn build(&self, scene: &mut Scene, parent: NodeId, props: &Props) -> NodeId {
        dropdown(
            scene,
            parent,
            Spec {
                options: props.text_list("options"),
                current: props.index("current"),
                placeholder: props.text("placeholder"),
                open: props.bool("open"),
                overlay: props.node("overlay"),
                overlay_size: (
                    props.number("overlay_width").get_untracked(),
                    props.number("overlay_height").get_untracked(),
                ),
                x: props.number("x"),
                y: props.number("y"),
                menu_x: props.number("menu_x"),
                menu_y: props.number("menu_y"),
                width: props.number("width").get_untracked(),
                height: props.number("height").get_untracked(),
                scale: props.number("scale").get_untracked(),
                radius: props.number("radius"),
                font: props.text("font"),
                font_size: props.number("font_size").get_untracked(),
                icon_size: props.number("icon_size").get_untracked(),
                accent: props.color("accent"),
                base: props.color("base"),
                hover: props.color("hover"),
                menu: props.color("menu"),
                border: props.color("border"),
                text: props.color("text"),
                active_text: props.color("active_text"),
                sound: None,
                on_select: props.index_event("on_select"),
            },
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn list_heights_follow_the_rows_and_cap_at_250() {
        assert_eq!(content_height(0), 0.0);
        assert_eq!(content_height(1), 32.0);
        assert_eq!(content_height(3), 100.0);
        assert_eq!(list_height(3), 100.0);
        assert_eq!(list_height(20), 250.0);
    }

    #[test]
    fn seven_rows_are_staggered_and_the_rest_show_at_once() {
        assert_eq!(visible_count(3), 3);
        assert_eq!(visible_count(30), 7);
        assert_eq!(stagger_ms(0), 20.0);
        assert_eq!(stagger_ms(2), 100.0);
        assert!(!shown(10.0, 0, 3));
        assert!(shown(20.0, 0, 3));
        assert!(!shown(60.0, 2, 3));
        assert!(shown(100.0, 2, 3));
        assert!(shown(0.0, 9, 30));
        assert_eq!(stagger_total_ms(3), 101.0);
    }

    #[test]
    fn fuzzy_search_ranks_word_starts_and_runs() {
        assert_eq!(fuzzy_score("", "x"), Some(1));
        assert_eq!(fuzzy_score("abc", "ab"), None);
        assert_eq!(fuzzy_score("xz", "abc"), None);
        assert!(fuzzy_score("fb", "foo_bar") > fuzzy_score("fb", "fobbar"));
        let options: Vec<String> = ["Spotify", "Firefox", "Fish tank", "mpv"].iter().map(|s| s.to_string()).collect();
        assert_eq!(filter(&options, ""), vec![0, 1, 2, 3]);
        assert_eq!(filter(&options, "fi"), vec![1, 2]);
        assert_eq!(filter(&options, "ZZ"), Vec::<usize>::new());
    }

    #[test]
    fn radii_are_clamped_like_the_qml() {
        assert_eq!(menu_radius(24.0), 16.0);
        assert_eq!(menu_radius(10.0), 10.0);
        assert_eq!(row_radius(10.0), 6.0);
        assert_eq!(row_radius(1.0), 2.0);
        assert_eq!(row_radius(40.0), 16.0);
    }

    #[test]
    fn the_registry_builds_a_dropdown_over_its_overlay() {
        let mut registry = crate::component::Registry::new();
        registry.register(DropdownComponent);
        let mut scene = Scene::new(acanthus_scene::Fonts::new());
        let root = scene.root();
        let mut given: Vec<(String, Prop)> = ["accent", "base", "hover", "menu", "border", "text", "active_text"]
            .map(|name| (name.to_owned(), Prop::color(Color::hex(0x336699))))
            .to_vec();
        given.push(("overlay".to_owned(), Prop::Node(root)));
        given.push(("options".to_owned(), Prop::TextList(Signal::new(vec!["one".to_owned(), "two".to_owned()]))));
        given.push(("overlay_width".to_owned(), Prop::number(400.0)));
        given.push(("overlay_height".to_owned(), Prop::number(300.0)));
        let node = registry.build("Dropdown", &mut scene, root, given).unwrap();
        assert!(scene.get(node).is_some());
        let err =
            registry.build("Dropdown", &mut scene, root, vec![("overlay".to_owned(), Prop::number(1.0))]).unwrap_err();
        assert_eq!(err.to_string(), "`Dropdown.overlay` expects a node, got a number");
    }
}
