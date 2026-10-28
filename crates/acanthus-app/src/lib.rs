//! App framework: a module registry with ordered start, a typed map modules use to hand each
//! other values, and [`Slot`], the building block of demand-started shared services.

pub mod actions;
pub mod dismiss;
pub mod menu;
mod module;
pub mod popup;
mod slot;
pub mod testing;

pub use actions::{Actions, Verb};
pub use module::{Module, Provided, Running, start_all};
pub use popup::Popup;
pub use slot::Slot;
