use std::cell::{Cell, RefCell};
use std::rc::Rc;

use acanthus_anim::{Action, AnimHandle, Loop, Pause, Sequential};
use acanthus_core::{Color, Effect, Signal};

use crate::dynamic::Host;
use crate::keyboard::{Clipboard, CursorShape, Key, KeyEvent, PointerEvent, PointerPhase};
use crate::node::{Node, NodeId};
use crate::scene::Scene;
use crate::text::{Fonts, ShapedText, TextParams, WrapMode};

const PASSWORD_CHARACTER: char = '\u{25CF}';
const DOUBLE_CLICK_SECONDS: f32 = 0.4;
const BLINK_HALF_PERIOD_MS: u32 = 500;
const CARET_WIDTH: f32 = 1.0;

type Validator = Box<dyn Fn(&str) -> bool>;

type TextCallback = RefCell<Option<Box<dyn FnMut(&str)>>>;

type KeyFilter = RefCell<Option<Box<dyn FnMut(&KeyEvent) -> bool>>>;

struct Inner {
    text: Signal<String>,
    cursor: Signal<usize>,
    anchor: Signal<usize>,
    password: Signal<bool>,
    read_only: Signal<bool>,
    max_length: Signal<usize>,
    blink: Signal<bool>,
    scroll: Signal<f32>,
    positions: RefCell<Vec<f32>>,
    on_accepted: TextCallback,
    on_edited: TextCallback,
    before_key: KeyFilter,
    validator: RefCell<Option<Validator>>,
    last_click: Cell<Option<(f32, u32)>>,
}

/// Editing state of a single-line text field: text, cursor, selection, scroll offset and callbacks. Indices count characters, not bytes. Cloning shares the state.
#[derive(Clone)]
pub struct TextInput {
    inner: Rc<Inner>,
}

impl Default for TextInput {
    fn default() -> Self {
        TextInput::new("")
    }
}

impl TextInput {
    /// Creates a field holding `text` with the cursor at the end.
    pub fn new(text: impl Into<String>) -> Self {
        let text: String = text.into();
        let end = text.chars().count();
        TextInput {
            inner: Rc::new(Inner {
                text: Signal::new(text),
                cursor: Signal::new(end),
                anchor: Signal::new(end),
                password: Signal::new(false),
                read_only: Signal::new(false),
                max_length: Signal::new(0),
                blink: Signal::new(false),
                scroll: Signal::new(0.0),
                positions: RefCell::new(Vec::new()),
                on_accepted: RefCell::new(None),
                on_edited: RefCell::new(None),
                before_key: RefCell::new(None),
                validator: RefCell::new(None),
                last_click: Cell::new(None),
            }),
        }
    }

    /// Sets whether the text is shown masked.
    pub fn password(self, v: bool) -> Self {
        self.inner.password.set(v);
        self
    }

    /// Sets whether edits are refused.
    pub fn read_only(self, v: bool) -> Self {
        self.inner.read_only.set(v);
        self
    }

    /// Sets the largest number of characters, zero for unlimited.
    pub fn max_length(self, v: usize) -> Self {
        self.inner.max_length.set(v);
        self
    }

    /// Sets the callback run with the text when Enter is pressed.
    pub fn on_accepted(self, f: impl FnMut(&str) + 'static) -> Self {
        *self.inner.on_accepted.borrow_mut() = Some(Box::new(f));
        self
    }

    /// Sets a filter that sees key events first and returns true to consume them.
    pub fn before_key(self, f: impl FnMut(&KeyEvent) -> bool + 'static) -> Self {
        *self.inner.before_key.borrow_mut() = Some(Box::new(f));
        self
    }

    /// Sets the callback run with the new text after every accepted edit.
    pub fn on_edited(self, f: impl FnMut(&str) + 'static) -> Self {
        *self.inner.on_edited.borrow_mut() = Some(Box::new(f));
        self
    }

    /// Sets a validator; edits whose result it rejects are refused.
    pub fn validator(self, f: impl Fn(&str) -> bool + 'static) -> Self {
        *self.inner.validator.borrow_mut() = Some(Box::new(f));
        self
    }

    /// The text signal.
    pub fn text(&self) -> Signal<String> {
        self.inner.text.clone()
    }

    /// The cursor position signal, in characters.
    pub fn cursor(&self) -> Signal<usize> {
        self.inner.cursor.clone()
    }

    /// The selection anchor signal; equal to the cursor when nothing is selected.
    pub fn anchor(&self) -> Signal<usize> {
        self.inner.anchor.clone()
    }

    /// The signal telling whether the text is masked.
    pub fn password_signal(&self) -> Signal<bool> {
        self.inner.password.clone()
    }

    /// The horizontal scroll offset signal, in logical pixels.
    pub fn scroll(&self) -> Signal<f32> {
        self.inner.scroll.clone()
    }

    /// The number of characters.
    pub fn len(&self) -> usize {
        self.inner.text.with(|t| t.chars().count())
    }

    /// True when the text is empty.
    pub fn is_empty(&self) -> bool {
        self.inner.text.with(|t| t.is_empty())
    }

    /// The selection as an ordered `(start, end)` pair.
    pub fn selection(&self) -> (usize, usize) {
        let (c, a) = (self.inner.cursor.get_untracked(), self.inner.anchor.get_untracked());
        (c.min(a), c.max(a))
    }

    /// True when at least one character is selected.
    pub fn has_selection(&self) -> bool {
        let (s, e) = self.selection();
        s != e
    }

    /// The selected text.
    pub fn selected_text(&self) -> String {
        let (s, e) = self.selection();
        self.inner.text.with(|t| t.chars().skip(s).take(e - s).collect())
    }

    /// Replaces the text without validation or callbacks and puts the cursor at the end.
    pub fn set_text(&self, text: impl Into<String>) {
        let text: String = text.into();
        let end = text.chars().count();
        self.inner.text.set(text);
        self.inner.cursor.set(end);
        self.inner.anchor.set(end);
    }

    /// Clears the text.
    pub fn clear(&self) {
        self.set_text("");
    }

    /// Selects everything.
    pub fn select_all(&self) {
        self.inner.anchor.set(0);
        self.inner.cursor.set(self.len());
    }

    /// Selects from `anchor` to `cursor`, clamped to the text.
    pub fn select(&self, anchor: usize, cursor: usize) {
        let len = self.len();
        self.inner.anchor.set(anchor.min(len));
        self.inner.cursor.set(cursor.min(len));
    }

    /// Moves the cursor to `index`, keeping the anchor when `extend` is true so the selection grows.
    pub fn set_cursor(&self, index: usize, extend: bool) {
        let index = index.min(self.len());
        self.inner.cursor.set(index);
        if !extend {
            self.inner.anchor.set(index);
        }
    }

    fn chars(&self) -> Vec<char> {
        self.inner.text.with(|t| t.chars().collect())
    }

    fn commit(&self, chars: &[char], cursor: usize) -> bool {
        let text: String = chars.iter().collect();
        if let Some(validator) = self.inner.validator.borrow().as_ref()
            && !validator(&text)
        {
            return false;
        }
        self.inner.text.set(text.clone());
        self.inner.cursor.set(cursor);
        self.inner.anchor.set(cursor);
        if let Some(f) = self.inner.on_edited.borrow_mut().as_mut() {
            f(&text);
        }
        true
    }

    /// Inserts `text` over the selection, dropping control characters and respecting the maximum length and validator. Returns true when the text changed.
    pub fn insert(&self, text: &str) -> bool {
        if self.inner.read_only.get_untracked() {
            return false;
        }
        let filtered: Vec<char> = text.chars().filter(|c| !c.is_control()).collect();
        if filtered.is_empty() {
            return false;
        }
        let mut chars = self.chars();
        let (start, end) = self.selection();
        let max = self.inner.max_length.get_untracked();
        let room = if max == 0 { usize::MAX } else { max.saturating_sub(chars.len() - (end - start)) };
        let filtered: Vec<char> = filtered.into_iter().take(room).collect();
        if filtered.is_empty() {
            return false;
        }
        let caret = start + filtered.len();
        chars.splice(start..end, filtered);
        self.commit(&chars, caret)
    }

    /// Deletes the selection. Returns true when the text changed.
    pub fn delete_selection(&self) -> bool {
        if self.inner.read_only.get_untracked() || !self.has_selection() {
            return false;
        }
        let (start, end) = self.selection();
        let mut chars = self.chars();
        chars.drain(start..end);
        self.commit(&chars, start)
    }

    fn backspace(&self, word: bool) -> bool {
        if self.inner.read_only.get_untracked() {
            return false;
        }
        if self.has_selection() {
            return self.delete_selection();
        }
        let cursor = self.inner.cursor.get_untracked();
        if cursor == 0 {
            return true;
        }
        let chars = self.chars();
        let from = if word { word_start(&chars, cursor) } else { cursor - 1 };
        let mut chars = chars;
        chars.drain(from..cursor);
        self.commit(&chars, from)
    }

    fn delete_forward(&self, word: bool) -> bool {
        if self.inner.read_only.get_untracked() {
            return false;
        }
        if self.has_selection() {
            return self.delete_selection();
        }
        let cursor = self.inner.cursor.get_untracked();
        let mut chars = self.chars();
        if cursor >= chars.len() {
            return true;
        }
        let to = if word { word_end(&chars, cursor) } else { cursor + 1 };
        chars.drain(cursor..to);
        self.commit(&chars, cursor)
    }

    fn copy(&self, clipboard: &Clipboard) -> bool {
        if self.inner.password.get_untracked() {
            return true;
        }
        if self.has_selection() {
            clipboard.set(self.selected_text());
        }
        true
    }

    fn cut(&self, clipboard: &Clipboard) -> bool {
        if self.inner.password.get_untracked() {
            return true;
        }
        if self.has_selection() && !self.inner.read_only.get_untracked() {
            clipboard.set(self.selected_text());
            self.delete_selection();
        }
        true
    }

    fn paste(&self, clipboard: &Clipboard) -> bool {
        let text = clipboard.get();
        self.insert(&text);
        true
    }

    /// Handles an editing key (characters, arrows, Home, End, Backspace, Delete, Enter and the clipboard shortcuts). Returns true when the key was consumed.
    pub fn handle_key(&self, event: &KeyEvent, clipboard: &Clipboard) -> bool {
        if let Some(f) = self.inner.before_key.borrow_mut().as_mut()
            && f(event)
        {
            return true;
        }
        let shift = event.modifiers.shift;
        let cursor = self.inner.cursor.get_untracked();
        let word = event.modifiers.ctrl;
        match event.key {
            Key::Char(c) if event.command() => match c.to_ascii_lowercase() {
                'a' => {
                    self.select_all();
                    true
                }
                'c' => self.copy(clipboard),
                'x' => self.cut(clipboard),
                'v' => self.paste(clipboard),
                _ => false,
            },
            Key::Char(_) if event.modifiers.alt => false,
            Key::Char(_) => {
                self.insert(&event.text);
                true
            }
            Key::Enter => {
                let text = self.inner.text.get_untracked();
                if let Some(f) = self.inner.on_accepted.borrow_mut().as_mut() {
                    f(&text);
                }
                true
            }
            Key::Backspace => self.backspace(word),
            Key::Delete => self.delete_forward(word),
            Key::Insert if shift => self.paste(clipboard),
            Key::Left => {
                if self.has_selection() && !shift && !word {
                    let (start, _) = self.selection();
                    self.set_cursor(start, false);
                } else {
                    let target = if word { word_start(&self.chars(), cursor) } else { cursor.saturating_sub(1) };
                    self.set_cursor(target, shift);
                }
                true
            }
            Key::Right => {
                if self.has_selection() && !shift && !word {
                    let (_, end) = self.selection();
                    self.set_cursor(end, false);
                } else {
                    let target = if word { word_end(&self.chars(), cursor) } else { cursor + 1 };
                    self.set_cursor(target, shift);
                }
                true
            }
            Key::Home => {
                self.set_cursor(0, shift);
                true
            }
            Key::End => {
                self.set_cursor(self.len(), shift);
                true
            }
            _ => false,
        }
    }

    /// The character index closest to the horizontal position `x` within the field.
    pub fn index_at(&self, x: f32) -> usize {
        let positions = self.inner.positions.borrow();
        let x = x + self.inner.scroll.get_untracked();
        positions
            .iter()
            .enumerate()
            .min_by(|a, b| (a.1 - x).abs().total_cmp(&(b.1 - x).abs()))
            .map_or(self.len(), |(i, _)| i)
    }

    fn pointer(&self, event: PointerEvent) {
        let index = self.index_at(event.position.x);
        match event.phase {
            PointerPhase::Press => {
                let clicks = match self.inner.last_click.get() {
                    Some((t, n)) if event.time - t <= DOUBLE_CLICK_SECONDS => n + 1,
                    _ => 1,
                };
                self.inner.last_click.set(Some((event.time, clicks)));
                match clicks {
                    1 => self.set_cursor(index, false),
                    2 => {
                        let (start, end) = word_range(&self.chars(), index);
                        self.select(start, end);
                    }
                    _ => self.select_all(),
                }
            }
            PointerPhase::Drag => {
                if self.inner.last_click.get().is_some_and(|(_, n)| n == 1) {
                    self.set_cursor(index, true);
                }
            }
            PointerPhase::Release => {}
        }
    }
}

fn is_word_char(c: char) -> bool {
    !c.is_whitespace()
}

fn word_start(chars: &[char], from: usize) -> usize {
    let mut i = from.min(chars.len());
    while i > 0 && !is_word_char(chars[i - 1]) {
        i -= 1;
    }
    while i > 0 && is_word_char(chars[i - 1]) {
        i -= 1;
    }
    i
}

fn word_end(chars: &[char], from: usize) -> usize {
    let mut i = from.min(chars.len());
    while i < chars.len() && is_word_char(chars[i]) {
        i += 1;
    }
    while i < chars.len() && !is_word_char(chars[i]) {
        i += 1;
    }
    i
}

fn word_range(chars: &[char], at: usize) -> (usize, usize) {
    if chars.is_empty() {
        return (0, 0);
    }
    let probe = at.min(chars.len() - 1);
    let class = is_word_char(chars[probe]);
    let mut start = probe;
    while start > 0 && is_word_char(chars[start - 1]) == class {
        start -= 1;
    }
    let mut end = probe;
    while end < chars.len() && is_word_char(chars[end]) == class {
        end += 1;
    }
    (start, end)
}

/// Appearance of a text field.
pub struct TextInputStyle {
    /// Font size.
    pub font_size: Signal<f32>,
    /// Font weight.
    pub font_weight: Signal<u16>,
    /// Font family, empty for the default.
    pub font_family: Signal<String>,
    /// Text colour.
    pub color: Signal<Color>,
    /// Selection highlight colour.
    pub selection_color: Signal<Color>,
    /// Caret colour.
    pub caret_color: Signal<Color>,
    /// Text shown while the field is empty.
    pub placeholder: Signal<String>,
    /// Colour of the placeholder.
    pub placeholder_color: Signal<Color>,
}

impl Default for TextInputStyle {
    fn default() -> Self {
        TextInputStyle {
            font_size: Signal::new(14.0),
            font_weight: Signal::new(400),
            font_family: Signal::new(String::new()),
            color: Signal::new(Color::BLACK),
            selection_color: Signal::new(Color::rgba(0.2, 0.45, 0.9, 0.5)),
            caret_color: Signal::new(Color::BLACK),
            placeholder: Signal::new(String::new()),
            placeholder_color: Signal::new(Color::rgba(0.5, 0.5, 0.5, 1.0)),
        }
    }
}

/// The nodes making up a text field created by [`Scene::text_input`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TextInputNodes {
    /// The clipping field node.
    pub field: NodeId,
    /// The text node.
    pub text: NodeId,
    /// The caret node.
    pub caret: NodeId,
    /// The selection highlight node.
    pub selection: NodeId,
    /// The placeholder node.
    pub placeholder: NodeId,
}

struct BlinkGuard {
    handle: Rc<Cell<Option<AnimHandle>>>,
}

impl Drop for BlinkGuard {
    fn drop(&mut self) {
        if let Some(handle) = self.handle.take() {
            handle.stop();
        }
    }
}

fn blink_effect(focus: Signal<bool>, blink: Signal<bool>) -> (Effect, BlinkGuard) {
    let slot: Rc<Cell<Option<AnimHandle>>> = Rc::new(Cell::new(None));
    let effect_slot = slot.clone();
    let effect = Effect::new(move || {
        if let Some(handle) = effect_slot.take() {
            handle.stop();
        }
        if focus.get() {
            blink.set(true);
            let on = blink.clone();
            let off = blink.clone();
            let cycle = Sequential::new()
                .then(Pause::new(BLINK_HALF_PERIOD_MS))
                .then(Action::new(move || off.set(false)))
                .then(Pause::new(BLINK_HALF_PERIOD_MS))
                .then(Action::new(move || on.set(true)));
            effect_slot.set(Some(acanthus_anim::start(Loop::infinite(cycle))));
        } else {
            blink.set(false);
        }
    });
    (effect, BlinkGuard { handle: slot })
}

fn mask(text: &str) -> String {
    text.chars().map(|_| PASSWORD_CHARACTER).collect()
}

struct TextInputHost {
    input: TextInput,
    fonts: Fonts,
    display: Signal<String>,
    style_size: Signal<f32>,
    style_weight: Signal<u16>,
    style_family: Signal<String>,
    focus: Signal<bool>,
    field: NodeId,
    text: NodeId,
    caret: NodeId,
    selection: NodeId,
    placeholder: NodeId,
    shaped: Option<ShapedText>,
}

impl Host for TextInputHost {
    fn sync(&mut self, scene: &mut Scene) -> bool {
        if !scene.contains(self.field) {
            return false;
        }
        let params = TextParams {
            size: self.style_size.get(),
            weight: self.style_weight.get(),
            axes: 0,
            styled: false,
            max_width: None,
            wrap: WrapMode::NoWrap,
            elide: crate::text::Elide::None,
            max_lines: 0,
            line_height: None,
            letter_spacing: 0.0,
            generation: self.fonts.generation().get(),
        };
        let display = self.display.get();
        let mut system = self.fonts.system();
        match self.shaped.as_mut() {
            Some(shaped) => shaped.update(&mut system, &self.display, &self.style_family, params),
            None => {
                self.shaped = Some(ShapedText::new(&mut system, &self.display, &self.style_family, params));
            }
        }
        drop(system);
        let shaped = self.shaped.as_ref().expect("shaped above");
        let positions = shaped.caret_positions(&display);
        let line_height = shaped.size.y;

        let inner = &self.input.inner;
        let cursor = inner.cursor.get().min(positions.len().saturating_sub(1));
        let anchor = inner.anchor.get().min(positions.len().saturating_sub(1));
        let focused = self.focus.get();
        let blink = inner.blink.get();
        let caret_x = positions.get(cursor).copied().unwrap_or(0.0);
        let total = positions.last().copied().unwrap_or(0.0);

        let geometry = scene.geometry(self.field);
        let (width, height) = geometry.map_or((0.0, 0.0), |g| (g.w, g.h));
        let mut scroll = inner.scroll.get_untracked();
        if width > 0.0 {
            if caret_x - scroll > width - CARET_WIDTH {
                scroll = caret_x - width + CARET_WIDTH;
            }
            if caret_x - scroll < 0.0 {
                scroll = caret_x;
            }
            scroll = scroll.clamp(0.0, (total - width + CARET_WIDTH).max(0.0));
        }
        inner.scroll.set(scroll);
        *inner.positions.borrow_mut() = positions.clone();

        let top = ((height - line_height) / 2.0).max(0.0);
        let (start, end) = (cursor.min(anchor), cursor.max(anchor));
        let (sel_x, sel_end) =
            (positions.get(start).copied().unwrap_or(0.0), positions.get(end).copied().unwrap_or(0.0));

        let set = |scene: &mut Scene, id: NodeId, x: f32, y: f32, w: Option<f32>, h: Option<f32>| {
            let Some(node) = scene.nodes.get(id) else { return };
            node.x.set(x);
            node.y.set(y);
            if let (Some(signal), Some(w)) = (node.width.as_ref(), w) {
                signal.set(w);
            }
            if let (Some(signal), Some(h)) = (node.height.as_ref(), h) {
                signal.set(h);
            }
        };
        set(scene, self.text, -scroll, top, None, None);
        set(scene, self.placeholder, 0.0, top, None, None);
        set(scene, self.caret, caret_x - scroll, top, Some(CARET_WIDTH), Some(line_height));
        set(scene, self.selection, sel_x - scroll, top, Some(sel_end - sel_x), Some(line_height));
        if let Some(node) = scene.nodes.get(self.caret) {
            node.visible.set(focused && blink && start == end);
        }
        if let Some(node) = scene.nodes.get(self.selection) {
            node.visible.set(start != end);
        }
        true
    }
}

struct FocusWhen {
    node: NodeId,
    trigger: Signal<bool>,
    last: bool,
}

impl Host for FocusWhen {
    fn sync(&mut self, scene: &mut Scene) -> bool {
        if !scene.contains(self.node) {
            return false;
        }
        let on = self.trigger.get();
        if on != self.last {
            self.last = on;
            if on {
                scene.set_focus(Some(self.node));
            } else if scene.focused() == Some(self.node) {
                let mut at = scene.get(self.node).and_then(|n| n.parent());
                while let Some(id) = at {
                    if scene.get(id).is_some_and(|n| n.focusable) {
                        break;
                    }
                    at = scene.get(id).and_then(|n| n.parent());
                }
                scene.set_focus(at);
            }
        }
        true
    }
}

impl Scene {
    /// Focuses `node` whenever `trigger` turns true.
    pub fn focus_when(&mut self, node: NodeId, trigger: Signal<bool>) {
        self.push_host(Box::new(FocusWhen { node, trigger, last: false }));
    }

    /// Builds a text field under `parent`: the `field` node is made focusable and wired to `input`, with text, selection, caret and placeholder nodes inside styled by `style`.
    pub fn text_input(
        &mut self,
        parent: NodeId,
        field: Node,
        input: &TextInput,
        style: TextInputStyle,
    ) -> TextInputNodes {
        let display = Signal::new(String::new());
        let (text, password) = (input.text(), input.password_signal());
        let display_effect = {
            let (text, password) = (text.clone(), password.clone());
            display.bind(move || if password.get() { mask(&text.get()) } else { text.get() })
        };

        let focus = field.active_focus();
        let (blink_effect, guard) = blink_effect(focus.clone(), input.inner.blink.clone());

        let clipboard = self.clipboard();
        let key_input = input.clone();
        let pointer_input = input.clone();
        let field = field
            .clip(true)
            .focusable(true)
            .cursor(CursorShape::Text)
            .on_key(move |event| key_input.handle_key(event, &clipboard))
            .on_pointer(move |event| pointer_input.pointer(event))
            .own(display_effect)
            .own(blink_effect)
            .own(guard);
        let field = self.add(parent, field);

        let placeholder_visible = Signal::new(true);
        let placeholder_effect = {
            let text = text.clone();
            placeholder_visible.bind(move || text.with(|t| t.is_empty()))
        };
        let placeholder = self.add(
            field,
            Node::text(style.placeholder.clone())
                .color(style.placeholder_color.clone())
                .font_size(style.font_size.clone())
                .font_weight(style.font_weight.clone())
                .font_family(style.font_family.clone())
                .visible(placeholder_visible)
                .own(placeholder_effect),
        );
        let selection = self.add(field, Node::rect().color(style.selection_color.clone()).size(0.0, 0.0));
        let text_node = self.add(
            field,
            Node::text(display.clone())
                .color(style.color.clone())
                .font_size(style.font_size.clone())
                .font_weight(style.font_weight.clone())
                .font_family(style.font_family.clone()),
        );
        let caret =
            self.add(field, Node::rect().color(style.caret_color.clone()).size(CARET_WIDTH, 0.0).visible(false));

        let fonts = self.fonts().clone();
        self.push_host(Box::new(TextInputHost {
            input: input.clone(),
            fonts,
            display,
            style_size: style.font_size,
            style_weight: style.font_weight,
            style_family: style.font_family,
            focus,
            field,
            text: text_node,
            caret,
            selection,
            placeholder,
            shaped: None,
        }));
        TextInputNodes { field, text: text_node, caret, selection, placeholder }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::keyboard::Modifiers;
    use crate::node::Kind;
    use crate::{Fonts, MouseButton};
    use acanthus_core::Vec2;

    fn typed(input: &TextInput, clipboard: &Clipboard, text: &str) {
        for c in text.chars() {
            assert!(input.handle_key(&KeyEvent::new(Key::Char(c)), clipboard));
        }
    }

    fn press(key: Key, modifiers: Modifiers) -> KeyEvent {
        KeyEvent::new(key).with_modifiers(modifiers)
    }

    const CTRL: Modifiers = Modifiers { shift: false, ctrl: true, alt: false, logo: false };
    const SHIFT: Modifiers = Modifiers { shift: true, ctrl: false, alt: false, logo: false };
    const NONE: Modifiers = Modifiers { shift: false, ctrl: false, alt: false, logo: false };

    fn field_scene() -> (Scene, TextInput, TextInputNodes) {
        let mut s = Scene::new(Fonts::new());
        s.set_viewport(Vec2::new(200.0, 40.0), 1.0);
        let input = TextInput::new("");
        let root = s.root();
        let nodes = s.text_input(root, Node::item().size(100.0, 30.0), &input, TextInputStyle::default());
        s.frame();
        (s, input, nodes)
    }

    #[test]
    fn focus_follows_the_trigger() {
        let (mut s, _input, nodes) = field_scene();
        let trigger = Signal::new(false);
        s.focus_when(nodes.field, trigger.clone());
        s.frame();
        assert_eq!(s.focused(), None);
        trigger.set(true);
        s.frame();
        assert_eq!(s.focused(), Some(nodes.field));
        trigger.set(false);
        s.frame();
        assert_eq!(s.focused(), None);
    }

    #[test]
    fn typing_inserts_at_the_cursor_and_backspace_removes() {
        let clipboard = Clipboard::new();
        let input = TextInput::new("");
        typed(&input, &clipboard, "helo");
        input.set_cursor(3, false);
        typed(&input, &clipboard, "l");
        assert_eq!(input.text().get_untracked(), "hello");
        assert_eq!(input.cursor().get_untracked(), 4);
        input.handle_key(&press(Key::Backspace, NONE), &clipboard);
        assert_eq!(input.text().get_untracked(), "helo");
        input.handle_key(&press(Key::Delete, NONE), &clipboard);
        assert_eq!(input.text().get_untracked(), "hel");
        input.handle_key(&press(Key::Home, NONE), &clipboard);
        input.handle_key(&press(Key::Backspace, NONE), &clipboard);
        assert_eq!(input.text().get_untracked(), "hel");
    }

    #[test]
    fn selection_is_replaced_by_typing_and_extended_with_shift() {
        let clipboard = Clipboard::new();
        let input = TextInput::new("hello world");
        input.handle_key(&press(Key::Home, NONE), &clipboard);
        for _ in 0..5 {
            input.handle_key(&press(Key::Right, SHIFT), &clipboard);
        }
        assert_eq!(input.selected_text(), "hello");
        typed(&input, &clipboard, "bye");
        assert_eq!(input.text().get_untracked(), "bye world");
        assert!(!input.has_selection());
    }

    #[test]
    fn before_key_claims_keys_first() {
        let clipboard = Clipboard::new();
        let input = TextInput::new("ab").before_key(|event| event.key == Key::Left);
        assert!(input.handle_key(&press(Key::Left, NONE), &clipboard));
        assert_eq!(input.cursor().get_untracked(), 2);
        assert!(input.handle_key(&press(Key::Right, NONE), &clipboard));
    }

    #[test]
    fn word_navigation_and_deletion() {
        let clipboard = Clipboard::new();
        let input = TextInput::new("one two three");
        input.handle_key(&press(Key::Left, CTRL), &clipboard);
        assert_eq!(input.cursor().get_untracked(), 8);
        input.handle_key(&press(Key::Backspace, CTRL), &clipboard);
        assert_eq!(input.text().get_untracked(), "one three");
        input.handle_key(&press(Key::Home, NONE), &clipboard);
        input.handle_key(&press(Key::Delete, CTRL), &clipboard);
        assert_eq!(input.text().get_untracked(), "three");
    }

    #[test]
    fn clipboard_shortcuts_copy_cut_and_paste() {
        let clipboard = Clipboard::new();
        let input = TextInput::new("abc def");
        input.handle_key(&press(Key::Char('a'), CTRL), &clipboard);
        input.handle_key(&press(Key::Char('c'), CTRL), &clipboard);
        assert_eq!(clipboard.get(), "abc def");
        input.handle_key(&press(Key::Char('x'), CTRL), &clipboard);
        assert!(input.is_empty());
        input.handle_key(&press(Key::Char('v'), CTRL), &clipboard);
        input.handle_key(&press(Key::Char('v'), CTRL), &clipboard);
        assert_eq!(input.text().get_untracked(), "abc defabc def");
    }

    #[test]
    fn password_fields_never_copy_and_limits_apply() {
        let clipboard = Clipboard::new();
        clipboard.set("kept");
        let input = TextInput::new("secret").password(true).max_length(8);
        input.select_all();
        input.handle_key(&press(Key::Char('c'), CTRL), &clipboard);
        input.handle_key(&press(Key::Char('x'), CTRL), &clipboard);
        assert_eq!(clipboard.get(), "kept");
        assert_eq!(input.text().get_untracked(), "secret");
        input.set_cursor(6, false);
        typed(&input, &clipboard, "1234");
        assert_eq!(input.text().get_untracked(), "secret12");
    }

    #[test]
    fn validator_rejects_edits_and_enter_accepts() {
        let clipboard = Clipboard::new();
        let accepted = Rc::new(RefCell::new(String::new()));
        let seen = accepted.clone();
        let input = TextInput::new("")
            .validator(|t| t.chars().all(|c| c.is_ascii_digit()))
            .on_accepted(move |t| *seen.borrow_mut() = t.to_string());
        typed(&input, &clipboard, "1a2");
        assert_eq!(input.text().get_untracked(), "12");
        assert!(input.handle_key(&press(Key::Enter, NONE), &clipboard));
        assert_eq!(*accepted.borrow(), "12");
    }

    #[test]
    fn read_only_ignores_edits_but_moves_the_cursor() {
        let clipboard = Clipboard::new();
        let input = TextInput::new("fixed").read_only(true);
        typed(&input, &clipboard, "x");
        input.handle_key(&press(Key::Backspace, NONE), &clipboard);
        assert_eq!(input.text().get_untracked(), "fixed");
        input.handle_key(&press(Key::Left, NONE), &clipboard);
        assert_eq!(input.cursor().get_untracked(), 4);
    }

    #[test]
    fn clicking_focuses_the_field_and_keys_reach_it() {
        let (mut s, input, nodes) = field_scene();
        let active = s.get(nodes.field).unwrap().active_focus();
        assert!(!active.get_untracked());
        s.pointer_motion(Vec2::new(10.0, 10.0));
        s.pointer_button(MouseButton::Left, true);
        s.pointer_button(MouseButton::Left, false);
        assert_eq!(s.focused(), Some(nodes.field));
        assert!(active.get_untracked());
        assert!(s.key_event(&KeyEvent::new(Key::Char('h'))));
        assert!(s.key_event(&KeyEvent::new(Key::Char('i'))));
        assert_eq!(input.text().get_untracked(), "hi");
        s.set_window_focus(false);
        assert!(!active.get_untracked());
        s.set_window_focus(true);
        assert!(active.get_untracked());
    }

    #[test]
    fn unhandled_keys_bubble_to_ancestors_and_tab_cycles_focus() {
        let (mut s, _input, nodes) = field_scene();
        let second = TextInput::new("");
        let root = s.root();
        let second_nodes =
            s.text_input(root, Node::item().x(120.0).size(60.0, 30.0), &second, TextInputStyle::default());
        s.frame();
        let escaped = Rc::new(Cell::new(false));
        let flag = escaped.clone();
        s.get_mut(root).unwrap().key = Some(Box::new(move |e| {
            if e.is(Key::Escape) {
                flag.set(true);
                return true;
            }
            false
        }));
        s.set_focus(Some(nodes.field));
        assert!(s.key_event(&KeyEvent::new(Key::Escape)));
        assert!(escaped.get());
        assert!(s.key_event(&KeyEvent::new(Key::Tab)));
        assert_eq!(s.focused(), Some(second_nodes.field));
        assert!(s.key_event(&press(Key::Tab, SHIFT)));
        assert_eq!(s.focused(), Some(nodes.field));
    }

    #[test]
    fn pressing_elsewhere_clears_focus_and_removal_drops_it() {
        let (mut s, _input, nodes) = field_scene();
        s.set_focus(Some(nodes.field));
        s.pointer_motion(Vec2::new(150.0, 35.0));
        s.pointer_button(MouseButton::Left, true);
        s.pointer_button(MouseButton::Left, false);
        assert_eq!(s.focused(), None);
        s.set_focus(Some(nodes.field));
        s.remove(nodes.field);
        assert_eq!(s.focused(), None);
    }

    #[test]
    fn hovering_a_text_field_requests_the_text_cursor() {
        let (mut s, _input, _nodes) = field_scene();
        assert_eq!(s.cursor_shape(), CursorShape::Default);
        s.pointer_motion(Vec2::new(10.0, 10.0));
        assert_eq!(s.cursor_shape(), CursorShape::Text);
        s.pointer_motion(Vec2::new(150.0, 35.0));
        assert_eq!(s.cursor_shape(), CursorShape::Default);
    }

    #[test]
    fn caret_follows_the_cursor_and_hides_without_focus() {
        let (mut s, input, nodes) = field_scene();
        s.set_focus(Some(nodes.field));
        s.frame();
        let visible = |s: &Scene| s.get(nodes.caret).unwrap().visible.get_untracked();
        assert!(visible(&s));
        let x0 = s.get(nodes.caret).unwrap().x.get_untracked();
        input.set_text("hello");
        s.frame();
        let x1 = s.get(nodes.caret).unwrap().x.get_untracked();
        assert!(x1 > x0 + 10.0, "{x0} {x1}");
        input.set_cursor(0, false);
        s.frame();
        assert_eq!(s.get(nodes.caret).unwrap().x.get_untracked(), x0);
        s.set_focus(None);
        s.frame();
        assert!(!visible(&s));
    }

    #[test]
    fn selection_rect_spans_the_selected_glyphs() {
        let (mut s, input, nodes) = field_scene();
        input.set_text("hello");
        s.frame();
        let selection = |s: &Scene| {
            let n = s.get(nodes.selection).unwrap();
            (n.visible.get_untracked(), n.x.get_untracked(), n.width.as_ref().unwrap().get_untracked())
        };
        assert!(!selection(&s).0);
        input.select_all();
        s.frame();
        let (visible, x, w) = selection(&s);
        assert!(visible);
        assert_eq!(x, 0.0);
        assert!(w > 20.0);
        input.select(1, 3);
        s.frame();
        let (_, x, w2) = selection(&s);
        assert!(x > 0.0 && w2 < w);
    }

    #[test]
    fn long_text_scrolls_to_keep_the_caret_visible() {
        let (mut s, input, nodes) = field_scene();
        input.set_text("a rather long line of text that cannot fit");
        s.frame();
        s.frame();
        let scroll = input.scroll().get_untracked();
        assert!(scroll > 0.0);
        let caret = s.get(nodes.caret).unwrap().x.get_untracked();
        assert!((0.0..=100.0).contains(&caret), "{caret}");
        input.set_cursor(0, false);
        s.frame();
        assert_eq!(input.scroll().get_untracked(), 0.0);
    }

    #[test]
    fn clicks_place_the_cursor_and_double_click_selects_a_word() {
        let (mut s, input, _nodes) = field_scene();
        input.set_text("hello world");
        s.frame();
        let positions = input.inner.positions.borrow().clone();
        let target = positions[3] + 0.5;
        s.pointer_motion_at(Vec2::new(target, 15.0), 0.0);
        s.pointer_button_at(MouseButton::Left, true, 0.0);
        s.pointer_button_at(MouseButton::Left, false, 0.05);
        assert_eq!(input.cursor().get_untracked(), 3);
        s.pointer_button_at(MouseButton::Left, true, 0.2);
        s.pointer_button_at(MouseButton::Left, false, 0.25);
        assert_eq!(input.selected_text(), "hello");
        s.pointer_button_at(MouseButton::Left, true, 0.4);
        s.pointer_button_at(MouseButton::Left, false, 0.45);
        assert_eq!(input.selected_text(), "hello world");
    }

    #[test]
    fn dragging_extends_the_selection() {
        let (mut s, input, _nodes) = field_scene();
        input.set_text("hello world");
        s.frame();
        let positions = input.inner.positions.borrow().clone();
        s.pointer_motion_at(Vec2::new(positions[1], 15.0), 1.0);
        s.pointer_button_at(MouseButton::Left, true, 1.0);
        s.pointer_motion_at(Vec2::new(positions[4], 15.0), 1.1);
        s.pointer_button_at(MouseButton::Left, false, 1.2);
        assert_eq!(input.selected_text(), "ell");
    }

    #[test]
    fn password_display_is_masked_and_placeholder_tracks_emptiness() {
        let mut s = Scene::new(Fonts::new());
        s.set_viewport(Vec2::new(200.0, 40.0), 1.0);
        let input = TextInput::new("").password(true);
        let root = s.root();
        let style = TextInputStyle { placeholder: Signal::new("Password".into()), ..TextInputStyle::default() };
        let nodes = s.text_input(root, Node::item().size(100.0, 30.0), &input, style);
        s.frame();
        let placeholder = |s: &Scene| s.get(nodes.placeholder).unwrap().visible.get_untracked();
        assert!(placeholder(&s));
        input.set_text("abc");
        s.frame();
        assert!(!placeholder(&s));
        let Kind::Text(style) = &s.get(nodes.text).unwrap().kind else { panic!("text node") };
        assert_eq!(style.text.get_untracked(), "\u{25CF}\u{25CF}\u{25CF}");
        assert_eq!(input.text().get_untracked(), "abc");
    }
}
