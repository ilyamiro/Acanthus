use std::cell::RefCell;
use std::rc::Rc;

use acanthus_core::Color;
use smithay_client_toolkit::reexports::protocols::xdg::shell::client::xdg_toplevel::ResizeEdge as WireEdge;
use smithay_client_toolkit::shell::xdg::window::{WindowConfigure, WindowDecorations};

/// Who draws the frame around a toplevel window.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Decorations {
    /// Follow what the compositor asks for.
    #[default]
    Server,
    /// Ask for a frame drawn by the application; the scene covers the whole window.
    Client,
    /// No frame at all.
    None,
}

impl Decorations {
    pub(crate) fn wire(self) -> WindowDecorations {
        match self {
            Decorations::Server => WindowDecorations::RequestServer,
            Decorations::Client => WindowDecorations::RequestClient,
            Decorations::None => WindowDecorations::None,
        }
    }
}

/// Edge or corner a window is resized from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResizeEdge {
    /// Top edge.
    Top,
    /// Bottom edge.
    Bottom,
    /// Left edge.
    Left,
    /// Right edge.
    Right,
    /// Top left corner.
    TopLeft,
    /// Top right corner.
    TopRight,
    /// Bottom left corner.
    BottomLeft,
    /// Bottom right corner.
    BottomRight,
}

impl ResizeEdge {
    pub(crate) fn wire(self) -> WireEdge {
        match self {
            ResizeEdge::Top => WireEdge::Top,
            ResizeEdge::Bottom => WireEdge::Bottom,
            ResizeEdge::Left => WireEdge::Left,
            ResizeEdge::Right => WireEdge::Right,
            ResizeEdge::TopLeft => WireEdge::TopLeft,
            ResizeEdge::TopRight => WireEdge::TopRight,
            ResizeEdge::BottomLeft => WireEdge::BottomLeft,
            ResizeEdge::BottomRight => WireEdge::BottomRight,
        }
    }
}

/// Request an application makes to its toplevel window through [`crate::AppHandle::window_command`].
#[derive(Debug, Clone, PartialEq)]
pub enum WindowCommand {
    /// Replaces the title.
    SetTitle(String),
    /// Starts an interactive move with the pointer; needs a recent button press.
    Move,
    /// Starts an interactive resize from `edge`; needs a recent button press.
    Resize(ResizeEdge),
    /// Maximizes the window.
    Maximize,
    /// Restores a maximized window.
    Unmaximize,
    /// Makes the window fullscreen on its current output.
    Fullscreen,
    /// Leaves fullscreen.
    Unfullscreen,
    /// Minimizes the window.
    Minimize,
    /// Closes the window as if the user asked to.
    Close,
}

/// State of a toplevel window as the compositor reports it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct WindowState {
    /// The window is maximized.
    pub maximized: bool,
    /// The window is fullscreen.
    pub fullscreen: bool,
    /// The window has the user's focus.
    pub activated: bool,
    /// The window is being resized interactively.
    pub resizing: bool,
    /// The window is tiled against at least one edge.
    pub tiled: bool,
}

impl WindowState {
    pub(crate) fn from_configure(configure: &WindowConfigure) -> Self {
        WindowState {
            maximized: configure.is_maximized(),
            fullscreen: configure.is_fullscreen(),
            activated: configure.is_activated(),
            resizing: configure.is_resizing(),
            tiled: configure.is_tiled(),
        }
    }
}

pub(crate) type CloseHandler = Rc<RefCell<dyn FnMut()>>;

/// Properties of a toplevel window.
#[derive(Clone)]
pub struct WindowConfig {
    /// Title the compositor shows.
    pub title: String,
    /// Application identifier that ties the window to a desktop entry.
    pub app_id: String,
    /// Initial size in logical pixels.
    pub size: (u32, u32),
    /// Smallest size the window may have.
    pub min_size: Option<(u32, u32)>,
    /// Largest size the window may have.
    pub max_size: Option<(u32, u32)>,
    /// Who draws the frame.
    pub decorations: Decorations,
    /// Colour the window is cleared to before drawing.
    pub clear: Color,
    pub(crate) on_close: Option<CloseHandler>,
}

impl std::fmt::Debug for WindowConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WindowConfig")
            .field("title", &self.title)
            .field("app_id", &self.app_id)
            .field("size", &self.size)
            .field("min_size", &self.min_size)
            .field("max_size", &self.max_size)
            .field("decorations", &self.decorations)
            .finish_non_exhaustive()
    }
}

impl WindowConfig {
    /// Window titled `title`, 640 by 480, with compositor decorations and an opaque black clear colour.
    pub fn new(title: impl Into<String>) -> Self {
        WindowConfig {
            title: title.into(),
            app_id: String::new(),
            size: (640, 480),
            min_size: None,
            max_size: None,
            decorations: Decorations::Server,
            clear: Color::BLACK,
            on_close: None,
        }
    }

    /// Sets the application identifier.
    pub fn app_id(self, app_id: impl Into<String>) -> Self {
        WindowConfig { app_id: app_id.into(), ..self }
    }

    /// Sets the initial size.
    pub fn size(self, width: u32, height: u32) -> Self {
        WindowConfig { size: (width, height), ..self }
    }

    /// Sets the smallest size.
    pub fn min_size(self, width: u32, height: u32) -> Self {
        WindowConfig { min_size: Some((width, height)), ..self }
    }

    /// Sets the largest size.
    pub fn max_size(self, width: u32, height: u32) -> Self {
        WindowConfig { max_size: Some((width, height)), ..self }
    }

    /// Sets who draws the frame.
    pub fn decorations(self, decorations: Decorations) -> Self {
        WindowConfig { decorations, ..self }
    }

    /// Sets the clear colour.
    pub fn clear(self, clear: Color) -> Self {
        WindowConfig { clear, ..self }
    }

    /// Calls `handler` when the user asks to close the window; the window then stays open until the application closes it. Without a handler the window closes itself.
    pub fn on_close(self, handler: impl FnMut() + 'static) -> Self {
        WindowConfig { on_close: Some(Rc::new(RefCell::new(handler))), ..self }
    }
}

pub(crate) fn resolve_size(
    suggested: (Option<u32>, Option<u32>),
    current: (u32, u32),
    min: Option<(u32, u32)>,
    max: Option<(u32, u32)>,
) -> (u32, u32) {
    let mut width = suggested.0.unwrap_or(current.0);
    let mut height = suggested.1.unwrap_or(current.1);
    if let Some((min_w, min_h)) = min {
        width = width.max(min_w);
        height = height.max(min_h);
    }
    if let Some((max_w, max_h)) = max {
        if max_w > 0 {
            width = width.min(max_w);
        }
        if max_h > 0 {
            height = height.min(max_h);
        }
    }
    (width.max(1), height.max(1))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn suggested_size_wins_over_current() {
        assert_eq!(resolve_size((Some(800), Some(600)), (640, 480), None, None), (800, 600));
    }

    #[test]
    fn missing_suggestion_keeps_current_axis() {
        assert_eq!(resolve_size((None, Some(600)), (640, 480), None, None), (640, 600));
        assert_eq!(resolve_size((None, None), (640, 480), None, None), (640, 480));
    }

    #[test]
    fn size_is_clamped_to_limits() {
        let limits = (Some((300, 200)), Some((1000, 0)));
        assert_eq!(resolve_size((Some(100), Some(100)), (0, 0), limits.0, limits.1), (300, 200));
        assert_eq!(resolve_size((Some(4000), Some(4000)), (0, 0), limits.0, limits.1), (1000, 4000));
    }

    #[test]
    fn size_is_never_zero() {
        assert_eq!(resolve_size((None, None), (0, 0), None, None), (1, 1));
    }

    #[test]
    fn config_builder_sets_fields() {
        let config = WindowConfig::new("Notes")
            .app_id("notes")
            .size(800, 600)
            .min_size(320, 240)
            .decorations(Decorations::Client);
        assert_eq!(config.title, "Notes");
        assert_eq!(config.app_id, "notes");
        assert_eq!(config.size, (800, 600));
        assert_eq!(config.min_size, Some((320, 240)));
        assert_eq!(config.decorations, Decorations::Client);
        assert!(config.on_close.is_none());
    }

    #[test]
    fn close_handler_is_shared_between_clones() {
        let count = Rc::new(RefCell::new(0));
        let seen = count.clone();
        let config = WindowConfig::new("x").on_close(move || *seen.borrow_mut() += 1);
        let copy = config.clone();
        (copy.on_close.unwrap().borrow_mut())();
        (config.on_close.unwrap().borrow_mut())();
        assert_eq!(*count.borrow(), 2);
    }

    #[test]
    fn edges_map_to_wire_values() {
        assert_eq!(ResizeEdge::BottomRight.wire(), WireEdge::BottomRight);
        assert_eq!(ResizeEdge::Left.wire(), WireEdge::Left);
    }
}
