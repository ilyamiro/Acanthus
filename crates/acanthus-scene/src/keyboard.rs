use std::cell::RefCell;
use std::rc::Rc;

use acanthus_core::Vec2;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Key {
    Char(char),
    Enter,
    Escape,
    Backspace,
    Delete,
    Tab,
    Left,
    Right,
    Up,
    Down,
    Home,
    End,
    PageUp,
    PageDown,
    Insert,
    Function(u8),
    Other(u32),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Modifiers {
    pub shift: bool,
    pub ctrl: bool,
    pub alt: bool,
    pub logo: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeyEvent {
    pub key: Key,
    pub text: String,
    pub modifiers: Modifiers,
    pub repeat: bool,
}

impl KeyEvent {
    pub fn new(key: Key) -> Self {
        let text = match key {
            Key::Char(c) => c.to_string(),
            _ => String::new(),
        };
        KeyEvent { key, text, modifiers: Modifiers::default(), repeat: false }
    }

    pub fn with_modifiers(mut self, modifiers: Modifiers) -> Self {
        self.modifiers = modifiers;
        self
    }

    pub fn with_text(mut self, text: impl Into<String>) -> Self {
        self.text = text.into();
        self
    }

    pub fn is(&self, key: Key) -> bool {
        self.key == key
    }

    pub fn command(&self) -> bool {
        self.modifiers.ctrl || self.modifiers.logo
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CursorShape {
    #[default]
    Default,
    Pointer,
    Text,
    Crosshair,
    Wait,
    Progress,
    Help,
    Grab,
    Grabbing,
    Move,
    NotAllowed,
    ResizeHorizontal,
    ResizeVertical,
    ResizeAll,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum PointerPhase {
    Press,
    Drag,
    Release,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PointerEvent {
    pub phase: PointerPhase,
    pub position: Vec2,
    pub time: f32,
}

#[derive(Clone, Default)]
pub struct Clipboard {
    text: Rc<RefCell<String>>,
}

impl Clipboard {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn get(&self) -> String {
        self.text.borrow().clone()
    }

    pub fn set(&self, text: impl Into<String>) {
        *self.text.borrow_mut() = text.into();
    }
}
