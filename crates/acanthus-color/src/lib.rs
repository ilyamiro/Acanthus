//! Colour science for themes: sRGB, HCT, CAM16, tonal palettes, Material 3 schemes, image quantization and source-colour scoring.

#![deny(missing_docs)]

pub(crate) use std::collections::HashMap as Map;

pub(crate) type IndexMap<K, V> = indexmap::IndexMap<K, V>;

/// sRGB, linear RGB, XYZ and Lab colour types and conversions.
pub mod color;
/// WCAG-style contrast ratios between tones.
pub mod contrast;
/// Detection and correction of disliked hues.
pub mod dislike;
/// Material 3 dynamic colours and schemes.
pub mod dynamic_color;
/// Colour error type.
pub mod error;
/// Generation of schemes from source colours and images.
pub mod generate;
/// The HCT colour space.
pub mod hct;
/// Tonal palettes.
pub mod palette;
/// Image quantization.
pub mod quantize;
/// Material 3 static schemes.
pub mod scheme;
/// Source-colour scoring.
pub mod score;
/// Warm and cool colour harmonies.
pub mod temperature;
/// Numeric helpers.
pub mod utils;

pub use error::Error;
