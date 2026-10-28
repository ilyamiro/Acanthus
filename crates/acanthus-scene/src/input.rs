use acanthus_core::Signal;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MouseButton {
    Left,
    Right,
    Middle,
    Other(u32),
}

impl MouseButton {
    pub fn from_linux(code: u32) -> Self {
        match code {
            0x110 => MouseButton::Left,
            0x111 => MouseButton::Right,
            0x112 => MouseButton::Middle,
            other => MouseButton::Other(other),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct ScrollDelta {
    pub x: f32,
    pub y: f32,
}

#[derive(Default)]
pub struct MouseArea {
    pub hovered: Signal<bool>,
    pub pressed: Signal<bool>,
    pub(crate) on_click: Option<Box<dyn FnMut(MouseButton)>>,
    pub(crate) on_scroll: Option<Box<dyn FnMut(ScrollDelta)>>,
}

impl MouseArea {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn hovered(&self) -> Signal<bool> {
        self.hovered.clone()
    }

    pub fn pressed(&self) -> Signal<bool> {
        self.pressed.clone()
    }

    pub fn on_click(mut self, f: impl FnMut(MouseButton) + 'static) -> Self {
        self.on_click = Some(Box::new(f));
        self
    }

    pub fn on_scroll(mut self, f: impl FnMut(ScrollDelta) + 'static) -> Self {
        self.on_scroll = Some(Box::new(f));
        self
    }
}
