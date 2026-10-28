//! Image and icon loading for Acanthus: raster and SVG decoding, freedesktop icon theme lookup, fill modes and an asynchronous reference-counted [`ImageStore`].
#![deny(missing_docs)]

mod decode;
mod fit;
mod icons;
mod memory;
mod source;
mod store;

pub use decode::{Animation, DecodeError, Decoded, decode_file, decode_raster, decode_svg};
pub use fit::{FillMode, Placement, place};
pub use icons::IconLookup;
pub use memory::register as register_memory_image;
pub use source::ImageSource;
pub use store::{ImageHandle, ImageId, ImageStatus, ImageStore};
