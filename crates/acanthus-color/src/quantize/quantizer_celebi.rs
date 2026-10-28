use super::{Quantizer, QuantizerResult, QuantizerWsmeans, QuantizerWu};
use crate::color::Argb;
use std::vec::Vec;

#[derive(Default)]
pub struct QuantizerCelebi;

impl Quantizer for QuantizerCelebi {
    fn quantize(pixels: &[Argb], max_colors: usize) -> QuantizerResult {
        let wu_result = QuantizerWu::quantize(pixels, max_colors);

        QuantizerWsmeans::quantize(pixels, max_colors, &wu_result.color_to_count.into_keys().collect::<Vec<_>>())
    }
}
