//! GPU renderer for scene display lists: wgpu setup, glyph atlas, image textures, layer compositing and shader effects.

#![deny(missing_docs)]

mod atlas;
mod gpu;
mod images;
mod renderer;

pub use gpu::{Gpu, GpuError};
pub use renderer::{GpuContext, Renderer};
pub use wgpu;
