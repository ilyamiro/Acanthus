use acanthus_core::Signal;

/// A pointer button.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MouseButton {
    /// The primary button.
    Left,
    /// The secondary button.
    Right,
    /// The middle button, usually the wheel.
    Middle,
    /// Any other button, carrying its Linux input code.
    Other(u32),
}

impl MouseButton {
    /// Maps a Linux input event code (`BTN_LEFT`, `BTN_RIGHT`, `BTN_MIDDLE`) to a button.
    pub fn from_linux(code: u32) -> Self {
        match code {
            0x110 => MouseButton::Left,
            0x111 => MouseButton::Right,
            0x112 => MouseButton::Middle,
            other => MouseButton::Other(other),
        }
    }
}

/// A scroll step in logical pixels, positive `y` scrolling content down.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct ScrollDelta {
    /// Horizontal amount.
    pub x: f32,
    /// Vertical amount.
    pub y: f32,
}

/// Pointer handling attached to a node: hover and press state plus click and scroll callbacks.
#[derive(Default)]
pub struct MouseArea {
    /// True while the pointer is over the node.
    pub hovered: Signal<bool>,
    /// True while a button is held on the node.
    pub pressed: Signal<bool>,
    pub(crate) on_click: Option<Box<dyn FnMut(MouseButton)>>,
    pub(crate) on_scroll: Option<Box<dyn FnMut(ScrollDelta)>>,
}

impl MouseArea {
    /// Creates an area with no callbacks.
    pub fn new() -> Self {
        Self::default()
    }

    /// The hover signal, shared with the area.
    pub fn hovered(&self) -> Signal<bool> {
        self.hovered.clone()
    }

    /// The pressed signal, shared with the area.
    pub fn pressed(&self) -> Signal<bool> {
        self.pressed.clone()
    }

    /// Sets the callback run when a button is released over the node after being pressed on it.
    pub fn on_click(mut self, f: impl FnMut(MouseButton) + 'static) -> Self {
        self.on_click = Some(Box::new(f));
        self
    }

    /// Sets the callback run for scroll steps over the node.
    pub fn on_scroll(mut self, f: impl FnMut(ScrollDelta) + 'static) -> Self {
        self.on_scroll = Some(Box::new(f));
        self
    }
}
