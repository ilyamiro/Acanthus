//! Wayland platform layer: connection, layer-shell, popup and toplevel surfaces, outputs, input and the event loop that renders scenes into them.

#![deny(missing_docs)]
mod app;
mod capture;
mod clipboard;
mod clock;
mod control;
mod dnd;
mod idle_inhibit;
mod lazy;
mod lock;
mod output;
mod popup;
mod scale;
mod surface;
mod window;

pub use app::{App, AppError, AppHandle, State, SurfaceId};
pub use capture::{Capture, CaptureConfig, CaptureError, CaptureState};
pub use clipboard::{Selection, SystemClipboard};
pub use dnd::DragOutcome;
pub use lock::{LockConfig, LockPhase};
pub use output::OutputInfo;
pub use popup::{PopupConfig, PopupEdge};
pub use smithay_client_toolkit::reexports::protocols::xdg::shell::client::xdg_positioner::ConstraintAdjustment;
pub use smithay_client_toolkit::shell::wlr_layer::{Anchor, KeyboardInteractivity, Layer};
pub use surface::LayerConfig;
pub use window::{Decorations, ResizeEdge, WindowCommand, WindowConfig, WindowState};
