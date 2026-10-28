//! Menu and context menu: a column of actions, separators, check marks and submenu rows with
//! pointer and keyboard navigation. The widget reports its size and where a submenu row sits, so a
//! host can put it on an `xdg_popup` (and a submenu on a nested one) and let the compositor grab
//! provide the click-outside dismissal.

use std::rc::Rc;

use acanthus_anim::{Easing, Keep};
use acanthus_core::{Color, Effect, Signal, Vec2, untrack};
use acanthus_scene::{Anchors, CursorShape, Key, MouseArea, MouseButton, Node, NodeId, Scene};

use crate::timing::FAST;

/// Height of an action row, in logical pixels.
pub const ROW_HEIGHT: f32 = 28.0;
/// Height of a separator row, in logical pixels.
pub const SEPARATOR_HEIGHT: f32 = 9.0;
/// Padding between the frame and the rows, in logical pixels.
pub const PADDING: f32 = 4.0;
const CHECK_COLUMN: f32 = 22.0;
const SIDE_MARGIN: f32 = 10.0;
const GAP: f32 = 24.0;
const ARROW_COLUMN: f32 = 18.0;

/// One action row of a menu.
#[derive(Clone)]
pub struct MenuItem {
    /// Identifier handed to the activation callback.
    pub id: String,
    /// Text of the row.
    pub label: Signal<String>,
    /// Whether the row reacts; a disabled row is dimmed and skipped by the keyboard.
    pub enabled: Signal<bool>,
    /// A check mark column entry: `Some(true)` draws a mark, `Some(false)` an empty slot, `None` no slot content.
    pub checked: Option<Signal<bool>>,
    /// Shortcut hint drawn at the right edge.
    pub shortcut: Option<String>,
    /// Whether activating the row opens a submenu instead of running an action.
    pub submenu: bool,
}

impl MenuItem {
    /// An enabled action row.
    pub fn action(id: impl Into<String>, label: impl Into<String>) -> MenuItem {
        MenuItem {
            id: id.into(),
            label: Signal::new(label.into()),
            enabled: Signal::new(true),
            checked: None,
            shortcut: None,
            submenu: false,
        }
    }

    /// An enabled row that opens a submenu.
    pub fn submenu(id: impl Into<String>, label: impl Into<String>) -> MenuItem {
        MenuItem { submenu: true, ..MenuItem::action(id, label) }
    }

    /// Adds a check mark column entry bound to `checked`.
    pub fn checked(self, checked: &Signal<bool>) -> MenuItem {
        MenuItem { checked: Some(checked.clone()), ..self }
    }

    /// Adds a shortcut hint.
    pub fn shortcut(self, shortcut: impl Into<String>) -> MenuItem {
        MenuItem { shortcut: Some(shortcut.into()), ..self }
    }

    /// Binds the enabled state to `enabled`.
    pub fn enabled(self, enabled: &Signal<bool>) -> MenuItem {
        MenuItem { enabled: enabled.clone(), ..self }
    }
}

/// A row of a menu.
#[derive(Clone)]
pub enum MenuEntry {
    /// An action or submenu row.
    Item(MenuItem),
    /// A thin line between groups.
    Separator,
}

/// What a key or activation asks the host to do.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    /// Nothing happens.
    None,
    /// Run the action with this id; the host usually closes the menu.
    Run(String),
    /// Open the submenu of the row at this index.
    OpenSubmenu(usize),
    /// Close this menu (Escape, or Left inside a submenu).
    Close,
}

fn selectable(entry: &MenuEntry) -> bool {
    matches!(entry, MenuEntry::Item(item) if item.enabled.get_untracked())
}

/// The next selectable row from `from` in direction `step` (+1 or -1), wrapping around; `None` when no row can be selected.
pub fn step(entries: &[MenuEntry], from: Option<usize>, step: i32) -> Option<usize> {
    let count = entries.len() as i32;
    if count == 0 {
        return None;
    }
    let mut at = match from {
        Some(index) => index as i32,
        None if step > 0 => -1,
        None => count,
    };
    for _ in 0..count {
        at = (at + step).rem_euclid(count);
        if selectable(&entries[at as usize]) {
            return Some(at as usize);
        }
    }
    None
}

/// The first or last selectable row.
pub fn edge(entries: &[MenuEntry], last: bool) -> Option<usize> {
    if last { step(entries, None, -1) } else { step(entries, None, 1) }
}

/// The next selectable row after `from` whose label starts with `typed` (case-insensitive), wrapping around.
pub fn typeahead(entries: &[MenuEntry], from: Option<usize>, typed: char) -> Option<usize> {
    let count = entries.len();
    let start = from.map_or(0, |index| index + 1);
    (0..count).map(|offset| (start + offset) % count.max(1)).find(|&index| match &entries[index] {
        MenuEntry::Item(item) if item.enabled.get_untracked() => {
            item.label.get_untracked().chars().next().is_some_and(|c| c.to_lowercase().eq(typed.to_lowercase()))
        }
        _ => false,
    })
}

/// What activating the row at `index` asks for.
pub fn activation(entries: &[MenuEntry], index: usize) -> Outcome {
    match entries.get(index) {
        Some(MenuEntry::Item(item)) if item.enabled.get_untracked() => {
            if item.submenu {
                Outcome::OpenSubmenu(index)
            } else {
                Outcome::Run(item.id.clone())
            }
        }
        _ => Outcome::None,
    }
}

/// Top edge and height of the row at `index` inside the menu, in logical pixels.
pub fn row_span(entries: &[MenuEntry], index: usize) -> (f32, f32) {
    let top = PADDING + entries[..index].iter().map(row_height).sum::<f32>();
    (top, row_height(&entries[index]))
}

fn row_height(entry: &MenuEntry) -> f32 {
    match entry {
        MenuEntry::Item(_) => ROW_HEIGHT,
        MenuEntry::Separator => SEPARATOR_HEIGHT,
    }
}

/// Colours and type of a menu.
pub struct MenuStyle {
    /// Fill of the frame.
    pub background: Signal<Color>,
    /// Colour of the frame border and the separators.
    pub border: Signal<Color>,
    /// Fill of the highlighted row.
    pub highlight: Signal<Color>,
    /// Text colour of enabled rows.
    pub text: Signal<Color>,
    /// Text colour of the shortcut hints and of disabled rows.
    pub muted: Signal<Color>,
    /// Text colour on the highlighted row.
    pub highlight_text: Signal<Color>,
    /// Font family.
    pub family: Signal<String>,
    /// Font size.
    pub font_size: f32,
    /// Corner radius of the frame.
    pub radius: f32,
    /// Narrowest width of the menu.
    pub min_width: f32,
}

/// Description of a menu.
pub struct MenuSpec {
    /// The rows from top to bottom.
    pub entries: Vec<MenuEntry>,
    /// Colours and type.
    pub style: MenuStyle,
    /// Called with the outcome of every activation, key and close request that asks the host for something.
    pub on_outcome: Rc<dyn Fn(Outcome)>,
}

/// A built menu.
pub struct Menu {
    /// The root node of the menu.
    pub node: NodeId,
    /// Size of the menu in logical pixels; it follows the measured labels, so a host sizes its popup from it.
    pub size: Signal<Vec2>,
    /// Index of the highlighted row.
    pub highlighted: Signal<Option<usize>>,
    /// The rows, for [`row_span`] and the activation helpers.
    pub entries: Rc<Vec<MenuEntry>>,
}

/// Builds the menu under `parent`.
pub fn menu(scene: &mut Scene, parent: NodeId, spec: MenuSpec) -> Menu {
    let MenuSpec { entries, style, on_outcome } = spec;
    let entries = Rc::new(entries);
    let mut keep = Keep::default();
    let highlighted: Signal<Option<usize>> = Signal::new(None);
    let height = PADDING * 2.0 + entries.iter().map(row_height).sum::<f32>();

    let mut widths = Vec::new();
    let mut rows = Vec::new();
    for (index, entry) in entries.iter().enumerate() {
        let MenuEntry::Item(item) = entry else {
            rows.push(None);
            continue;
        };
        let label_color = {
            let (enabled, text, muted, highlighted, on) = (
                item.enabled.clone(),
                style.text.clone(),
                style.muted.clone(),
                highlighted.clone(),
                style.highlight_text.clone(),
            );
            keep.derive(move || {
                if !enabled.get() {
                    muted.get()
                } else if highlighted.get() == Some(index) {
                    on.get()
                } else {
                    text.get()
                }
            })
        };
        let label = Node::text(&item.label)
            .font_family(&style.family)
            .font_size(style.font_size)
            .color(&label_color)
            .anchors(Anchors::default().left(CHECK_COLUMN + SIDE_MARGIN).v_center());
        let label_width = label.implicit_size();
        let shortcut = item.shortcut.as_ref().map(|text| {
            Node::text(Signal::new(text.clone()))
                .font_family(&style.family)
                .font_size(style.font_size)
                .color(&style.muted)
                .anchors(Anchors::default().right(SIDE_MARGIN + ARROW_COLUMN).v_center())
        });
        let shortcut_width = shortcut.as_ref().map(Node::implicit_size);
        let reserved = CHECK_COLUMN + SIDE_MARGIN * 2.0 + ARROW_COLUMN;
        widths.push(keep.derive(move || {
            let shortcut = shortcut_width.as_ref().map_or(0.0, |w| w.get().x + GAP);
            reserved + label_width.get().x + shortcut
        }));
        rows.push(Some((label, shortcut)));
    }
    let width = {
        let widths = widths.clone();
        let min = style.min_width;
        keep.derive(move || widths.iter().map(|w| w.get()).fold(min, f32::max) + PADDING * 2.0)
    };
    let size = {
        let width = width.clone();
        keep.derive(move || Vec2::new(width.get(), height))
    };

    let outer_entries = entries.clone();
    let key_highlight = highlighted.clone();
    let key_outcome = on_outcome.clone();
    let outer = Node::item()
        .width(&width)
        .height(height)
        .focusable(true)
        .on_key(move |event| {
            let current = key_highlight.get_untracked();
            let go = |to: Option<usize>| {
                if to.is_some() {
                    key_highlight.set(to);
                }
                true
            };
            match event.key {
                Key::Down => go(step(&outer_entries, current, 1)),
                Key::Up => go(step(&outer_entries, current, -1)),
                Key::Home => go(edge(&outer_entries, false)),
                Key::End => go(edge(&outer_entries, true)),
                Key::Enter | Key::Char(' ') | Key::Right => {
                    let Some(index) = current else { return event.key != Key::Right };
                    let outcome = activation(&outer_entries, index);
                    let submenu = matches!(outcome, Outcome::OpenSubmenu(_));
                    if event.key == Key::Right && !submenu {
                        return false;
                    }
                    key_outcome(outcome);
                    true
                }
                Key::Left | Key::Escape => {
                    key_outcome(Outcome::Close);
                    true
                }
                Key::Char(c) if !event.modifiers.ctrl && !event.modifiers.alt && !event.modifiers.logo => {
                    go(typeahead(&outer_entries, current, c))
                }
                _ => false,
            }
        })
        .own(keep);
    let outer = scene.add(parent, outer);
    scene.add(
        outer,
        Node::rect()
            .anchors(Anchors::fill(0.0))
            .radius(style.radius)
            .color(&style.background)
            .border(1.0, &style.border),
    );

    let mut top = PADDING;
    for (index, (entry, row)) in entries.iter().zip(rows).enumerate() {
        match (entry, row) {
            (MenuEntry::Separator, _) => {
                scene.add(
                    outer,
                    Node::rect()
                        .anchors(
                            Anchors::default()
                                .left(PADDING + SIDE_MARGIN * 0.5)
                                .right(PADDING + SIDE_MARGIN * 0.5)
                                .top(top + SEPARATOR_HEIGHT / 2.0),
                        )
                        .height(1.0)
                        .color(&style.border),
                );
                top += SEPARATOR_HEIGHT;
            }
            (MenuEntry::Item(item), Some((label, shortcut))) => {
                let mut row_keep = Keep::default();
                let area = MouseArea::new();
                let hovered = area.hovered();
                {
                    let (highlighted, hovered) = (highlighted.clone(), hovered.clone());
                    row_keep.add(Effect::new(move || {
                        if hovered.get() {
                            untrack(|| highlighted.set(Some(index)));
                        }
                    }));
                }
                let area = {
                    let (entries, on_outcome, highlighted) = (entries.clone(), on_outcome.clone(), highlighted.clone());
                    area.on_click(move |button| {
                        if button == MouseButton::Left {
                            highlighted.set(Some(index));
                            match activation(&entries, index) {
                                Outcome::None => {}
                                outcome => on_outcome(outcome),
                            }
                        }
                    })
                };
                let fill_target = {
                    let (highlighted, enabled, accent) =
                        (highlighted.clone(), item.enabled.clone(), style.highlight.clone());
                    row_keep.derive(move || {
                        if enabled.get() && highlighted.get() == Some(index) {
                            accent.get()
                        } else {
                            accent.get().with_alpha(0.0)
                        }
                    })
                };
                let fill = row_keep.ease(&fill_target, FAST, Easing::Linear);
                let mark = item
                    .checked
                    .clone()
                    .map(|checked| row_keep.derive(move || if checked.get() { "✓".to_owned() } else { String::new() }));
                let row_node = scene.add(
                    outer,
                    Node::item()
                        .anchors(Anchors::default().left(PADDING).right(PADDING).top(top))
                        .height(ROW_HEIGHT)
                        .mouse(area)
                        .cursor(CursorShape::Pointer)
                        .own(row_keep),
                );
                scene.add(row_node, Node::rect().anchors(Anchors::fill(0.0)).radius(4.0).color(&fill));
                if let Some(mark) = mark {
                    scene.add(
                        row_node,
                        Node::text(&mark)
                            .font_family(&style.family)
                            .font_size(style.font_size)
                            .color(&style.text)
                            .anchors(Anchors::default().left(SIDE_MARGIN).v_center()),
                    );
                }
                scene.add(row_node, label);
                if let Some(shortcut) = shortcut {
                    scene.add(row_node, shortcut);
                }
                if item.submenu {
                    scene.add(
                        row_node,
                        Node::text(Signal::new("▸".to_owned()))
                            .font_family(&style.family)
                            .font_size(style.font_size)
                            .color(&style.muted)
                            .anchors(Anchors::default().right(SIDE_MARGIN).v_center()),
                    );
                }
                top += ROW_HEIGHT;
            }
            (MenuEntry::Item(_), None) => {}
        }
    }
    Menu { node: outer, size, highlighted, entries }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;

    use acanthus_scene::{Fonts, KeyEvent};

    use super::*;

    fn entries() -> Vec<MenuEntry> {
        let off = Signal::new(false);
        vec![
            MenuEntry::Item(MenuItem::action("open", "Open")),
            MenuEntry::Item(MenuItem::action("copy", "Copy").enabled(&off)),
            MenuEntry::Separator,
            MenuEntry::Item(MenuItem::submenu("more", "More")),
            MenuEntry::Item(MenuItem::action("quit", "Quit").shortcut("Ctrl+Q")),
        ]
    }

    fn style() -> MenuStyle {
        MenuStyle {
            background: Signal::new(Color::hex(0x202020)),
            border: Signal::new(Color::hex(0x404040)),
            highlight: Signal::new(Color::hex(0x3060c0)),
            text: Signal::new(Color::WHITE),
            muted: Signal::new(Color::hex(0x888888)),
            highlight_text: Signal::new(Color::WHITE),
            family: Signal::new("Adwaita Mono".to_owned()),
            font_size: 13.0,
            radius: 8.0,
            min_width: 120.0,
        }
    }

    fn built() -> (Scene, Menu, Rc<RefCell<Vec<Outcome>>>) {
        let mut scene = Scene::new(Fonts::new());
        let root = scene.root();
        let seen = Rc::new(RefCell::new(Vec::new()));
        let sink = seen.clone();
        let menu = menu(
            &mut scene,
            root,
            MenuSpec { entries: entries(), style: style(), on_outcome: Rc::new(move |o| sink.borrow_mut().push(o)) },
        );
        scene.set_viewport(Vec2::new(300.0, 300.0), 1.0);
        scene.frame();
        (scene, menu, seen)
    }

    #[test]
    fn stepping_skips_separators_and_disabled_rows_and_wraps() {
        let rows = entries();
        assert_eq!(step(&rows, None, 1), Some(0));
        assert_eq!(step(&rows, Some(0), 1), Some(3));
        assert_eq!(step(&rows, Some(4), 1), Some(0));
        assert_eq!(step(&rows, Some(0), -1), Some(4));
        assert_eq!(step(&rows, None, -1), Some(4));
        assert_eq!(edge(&rows, false), Some(0));
        assert_eq!(edge(&rows, true), Some(4));
        assert_eq!(step(&[MenuEntry::Separator], None, 1), None);
        assert_eq!(step(&[], None, 1), None);
    }

    #[test]
    fn typeahead_finds_the_next_enabled_label() {
        let rows = entries();
        assert_eq!(typeahead(&rows, None, 'q'), Some(4));
        assert_eq!(typeahead(&rows, None, 'O'), Some(0));
        assert_eq!(typeahead(&rows, None, 'c'), None);
        assert_eq!(typeahead(&rows, Some(4), 'o'), Some(0));
    }

    #[test]
    fn activation_distinguishes_actions_submenus_and_dead_rows() {
        let rows = entries();
        assert_eq!(activation(&rows, 0), Outcome::Run("open".to_owned()));
        assert_eq!(activation(&rows, 1), Outcome::None);
        assert_eq!(activation(&rows, 2), Outcome::None);
        assert_eq!(activation(&rows, 3), Outcome::OpenSubmenu(3));
        assert_eq!(activation(&rows, 9), Outcome::None);
    }

    #[test]
    fn row_spans_stack_with_separators_thinner() {
        let rows = entries();
        assert_eq!(row_span(&rows, 0), (PADDING, ROW_HEIGHT));
        assert_eq!(row_span(&rows, 3), (PADDING + ROW_HEIGHT * 2.0 + SEPARATOR_HEIGHT, ROW_HEIGHT));
    }

    #[test]
    fn the_size_follows_the_rows_and_labels() {
        let (_, menu, _) = built();
        let size = menu.size.get_untracked();
        assert_eq!(size.y, PADDING * 2.0 + ROW_HEIGHT * 4.0 + SEPARATOR_HEIGHT);
        assert!(size.x >= 120.0 + PADDING * 2.0);
    }

    #[test]
    fn keys_move_the_highlight_and_report_outcomes() {
        let (mut scene, menu, seen) = built();
        scene.set_focus(Some(menu.node));
        for key in [Key::Down, Key::Down, Key::Down] {
            assert!(scene.key_event(&KeyEvent::new(key)));
        }
        assert_eq!(menu.highlighted.get_untracked(), Some(4));
        scene.key_event(&KeyEvent::new(Key::Enter));
        scene.key_event(&KeyEvent::new(Key::Up));
        scene.key_event(&KeyEvent::new(Key::Right));
        scene.key_event(&KeyEvent::new(Key::Escape));
        assert_eq!(*seen.borrow(), vec![Outcome::Run("quit".to_owned()), Outcome::OpenSubmenu(3), Outcome::Close]);
    }

    #[test]
    fn right_on_a_plain_row_is_left_unhandled_and_enter_without_highlight_is_swallowed() {
        let (mut scene, menu, seen) = built();
        scene.set_focus(Some(menu.node));
        assert!(scene.key_event(&KeyEvent::new(Key::Enter)));
        menu.highlighted.set(Some(0));
        assert!(!scene.key_event(&KeyEvent::new(Key::Right)));
        assert!(seen.borrow().is_empty());
    }

    #[test]
    fn hovering_highlights_and_clicking_activates() {
        let (mut scene, menu, seen) = built();
        let (top, height) = row_span(&menu.entries, 4);
        let at = Vec2::new(40.0, top + height / 2.0);
        scene.pointer_motion_at(at, 0.0);
        assert_eq!(menu.highlighted.get_untracked(), Some(4));
        scene.pointer_button_at(MouseButton::Left, true, 0.1);
        scene.pointer_button_at(MouseButton::Left, false, 0.1);
        assert_eq!(*seen.borrow(), vec![Outcome::Run("quit".to_owned())]);
        let (disabled_top, disabled_height) = row_span(&menu.entries, 1);
        scene.pointer_motion_at(Vec2::new(40.0, disabled_top + disabled_height / 2.0), 0.2);
        scene.pointer_button_at(MouseButton::Left, true, 0.3);
        scene.pointer_button_at(MouseButton::Left, false, 0.3);
        assert_eq!(seen.borrow().len(), 1);
    }
}
