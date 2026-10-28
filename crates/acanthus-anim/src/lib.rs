//! Animation engine of Acanthus: Qt-exact easing curves, tweens, springs, sequences, and [`Animated`] signals driven by a per-thread driver.
//!
//! Start animations with [`start`] and advance them once per frame with [`tick`].
#![deny(missing_docs)]

mod animatable;
mod animations;
mod behavior;
mod driver;
mod easing;
mod keep;

pub use animatable::Animatable;
pub use animations::{Action, Animation, Loop, Parallel, Pause, Sequential, Spring, SpringParams, Tween};
pub use behavior::{Animated, Motion};
pub use driver::{AnimHandle, is_active, start, tick};
pub use easing::Easing;
pub use keep::{Keep, counted};
