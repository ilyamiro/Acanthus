//! Reactive core of the Acanthus engine: signals, effects and the small value types (colour, rectangle, vector) shared by every other crate.
//!
//! State lives in [`Signal`]s, derived state in [`Memo`]s and side effects in [`Effect`]s that re-run synchronously when a signal they read changes. The runtime is thread-local, so none of these types are `Send`.
#![deny(missing_docs)]

/// Editable list signal.
pub mod list;
/// Signals, effects, memos and batching.
pub mod reactive;
/// Colour, rectangle and scaling helpers.
pub mod types;
pub use list::ListSignal;
pub use reactive::{Effect, Memo, Scope, Signal, Tracker, batch, epoch, untrack};
pub use types::{Affine2, Color, Rect, Vec2, darken, lighten, scaled};
