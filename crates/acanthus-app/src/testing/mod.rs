//! Headless verification: render a scene offscreen through the real renderer, replay a pointer and key
//! script on a deterministic clock, block the thread on purpose and log frame times.
//!
//! Tests of examples, modules and probes use [`Headless`] instead of launching a window; it needs a
//! Vulkan adapter (lavapipe is enough) and reports [`HeadlessError::NoAdapter`] without one.

mod headless;
mod script;

pub use headless::{FRAME_MS, FrameLog, Headless, HeadlessError, Image, Run};
pub use script::{Action, Script, ScriptError, Step};
