use std::cell::RefCell;
use std::rc::Rc;

use acanthus_core::Vec2;

/// A logical key, independent of the keyboard layout where possible.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Key {
    /// A key that produces a character.
    Char(char),
    /// The Enter or Return key.
    Enter,
    /// The Escape key.
    Escape,
    /// The Backspace key.
    Backspace,
    /// The Delete key.
    Delete,
    /// The Tab key.
    Tab,
    /// Left arrow.
    Left,
    /// Right arrow.
    Right,
    /// Up arrow.
    Up,
    /// Down arrow.
    Down,
    /// The Home key.
    Home,
    /// The End key.
    End,
    /// The Page Up key.
    PageUp,
    /// The Page Down key.
    PageDown,
    /// The Insert key.
    Insert,
    /// A function key, `Function(1)` being F1.
    Function(u8),
    /// Any other key, carrying its keysym.
    Other(u32),
}

/// The modifier keys held during an event.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Modifiers {
    /// Shift is held.
    pub shift: bool,
    /// Control is held.
    pub ctrl: bool,
    /// Alt is held.
    pub alt: bool,
    /// The logo (super) key is held.
    pub logo: bool,
}

/// A key press delivered to the focused node.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeyEvent {
    /// The logical key.
    pub key: Key,
    /// The text the key produced, empty for non-text keys.
    pub text: String,
    /// The modifiers held.
    pub modifiers: Modifiers,
    /// True for auto-repeat events.
    pub repeat: bool,
}

impl KeyEvent {
    /// Creates an event for `key` with no text, no modifiers and no repeat.
    pub fn new(key: Key) -> Self {
        let text = match key {
            Key::Char(c) => c.to_string(),
            _ => String::new(),
        };
        KeyEvent { key, text, modifiers: Modifiers::default(), repeat: false }
    }

    /// Sets the modifiers.
    pub fn with_modifiers(mut self, modifiers: Modifiers) -> Self {
        self.modifiers = modifiers;
        self
    }

    /// Sets the produced text.
    pub fn with_text(mut self, text: impl Into<String>) -> Self {
        self.text = text.into();
        self
    }

    /// True when the event is for `key`.
    pub fn is(&self, key: Key) -> bool {
        self.key == key
    }

    /// True when the platform command modifier (control or logo) is held.
    pub fn command(&self) -> bool {
        self.modifiers.ctrl || self.modifiers.logo
    }
}

/// The mouse cursor image a node requests while hovered.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CursorShape {
    /// The default arrow.
    #[default]
    Default,
    /// A pointing hand.
    Pointer,
    /// A text insertion bar.
    Text,
    /// A crosshair.
    Crosshair,
    /// Busy.
    Wait,
    /// Busy but still interactive.
    Progress,
    /// Help available.
    Help,
    /// An open hand.
    Grab,
    /// A closed hand.
    Grabbing,
    /// Moving content.
    Move,
    /// The action is not allowed.
    NotAllowed,
    /// Horizontal resize arrows.
    ResizeHorizontal,
    /// Vertical resize arrows.
    ResizeVertical,
    /// Resize in all directions.
    ResizeAll,
}

/// The stage of a pointer gesture.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum PointerPhase {
    /// The button went down.
    Press,
    /// The pointer moved with the button down.
    Drag,
    /// The button was released.
    Release,
}

/// A pointer sample delivered to drag handlers.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PointerEvent {
    /// The gesture stage.
    pub phase: PointerPhase,
    /// The pointer position in scene coordinates.
    pub position: Vec2,
    /// The event time in seconds.
    pub time: f32,
}

/// Source and sink of clipboard text behind a [`Clipboard`], such as the desktop's selection.
pub trait ClipboardBackend {
    /// The text currently on the clipboard.
    fn text(&self) -> String;
    /// Puts `text` on the clipboard.
    fn set_text(&self, text: &str);
}

#[derive(Default)]
struct ClipboardInner {
    text: String,
    backend: Option<Rc<dyn ClipboardBackend>>,
}

/// Plain-text clipboard used by text widgets. Cloning shares the contents; with a backend the contents are the system clipboard's.
#[derive(Clone, Default)]
pub struct Clipboard {
    inner: Rc<RefCell<ClipboardInner>>,
}

impl Clipboard {
    /// Creates an empty clipboard.
    pub fn new() -> Self {
        Self::default()
    }

    /// Routes this clipboard and all its clones through `backend`.
    pub fn set_backend(&self, backend: Rc<dyn ClipboardBackend>) {
        self.inner.borrow_mut().backend = Some(backend);
    }

    /// The current text.
    pub fn get(&self) -> String {
        let backend = self.inner.borrow().backend.clone();
        match backend {
            Some(backend) => backend.text(),
            None => self.inner.borrow().text.clone(),
        }
    }

    /// Replaces the text.
    pub fn set(&self, text: impl Into<String>) {
        let text = text.into();
        let backend = self.inner.borrow().backend.clone();
        if let Some(backend) = backend {
            backend.set_text(&text);
        }
        self.inner.borrow_mut().text = text;
    }
}

#[cfg(test)]
mod clipboard_tests {
    use super::*;

    #[derive(Default)]
    struct Fake(RefCell<String>);

    impl ClipboardBackend for Fake {
        fn text(&self) -> String {
            self.0.borrow().clone()
        }
        fn set_text(&self, text: &str) {
            *self.0.borrow_mut() = text.to_owned();
        }
    }

    #[test]
    fn without_backend_text_stays_local() {
        let clipboard = Clipboard::new();
        clipboard.set("a");
        assert_eq!(clipboard.get(), "a");
    }

    #[test]
    fn backend_is_read_and_written_through_every_clone() {
        let backend = Rc::new(Fake::default());
        let clipboard = Clipboard::new();
        let other = clipboard.clone();
        clipboard.set_backend(backend.clone());
        other.set("copied");
        assert_eq!(backend.text(), "copied");
        *backend.0.borrow_mut() = "from outside".into();
        assert_eq!(clipboard.get(), "from outside");
    }
}
