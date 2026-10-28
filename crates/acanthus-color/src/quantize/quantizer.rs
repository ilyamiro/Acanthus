use crate::{IndexMap, color::Argb};

/// Reduces a set of pixels to a small palette.
pub trait Quantizer {
    /// Quantizes `pixels` into at most `max_colors` colours.
    fn quantize(pixels: &[Argb], max_colors: usize) -> QuantizerResult;
}

/// Output of a [`Quantizer`].
pub struct QuantizerResult {
    /// Population of every resulting colour.
    pub color_to_count: IndexMap<Argb, u32>,
    /// Maps input pixels to the cluster colour they were assigned to (empty when the quantizer does not track it).
    pub input_pixel_to_cluster_pixel: IndexMap<Argb, Argb>,
}
