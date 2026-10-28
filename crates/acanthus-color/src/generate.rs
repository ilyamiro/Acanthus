use crate::color::Argb;
use crate::dynamic_color::{DynamicScheme, Variant};
use crate::hct::{Cam16, Hct};
use crate::quantize::{Quantizer, QuantizerCelebi};
use crate::scheme::Scheme;
use crate::score::Score;

pub const IMAGE_SAMPLE_SIZE: u32 = 112;
pub const SMART_THUMBNAIL_SIZE: u32 = 128;
const MAX_COLORS: usize = 128;
const MIN_CHROMA: f64 = 5.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SchemeType {
    Content,
    Expressive,
    Fidelity,
    FruitSalad,
    Monochrome,
    Neutral,
    Rainbow,
    #[default]
    TonalSpot,
    Vibrant,
    Smart,
}

impl SchemeType {
    pub fn parse(name: &str) -> Option<SchemeType> {
        let name = name.trim().to_lowercase();
        let name = name.strip_prefix("scheme-").unwrap_or(&name);
        Some(match name {
            "content" => SchemeType::Content,
            "expressive" => SchemeType::Expressive,
            "fidelity" => SchemeType::Fidelity,
            "fruit-salad" => SchemeType::FruitSalad,
            "monochrome" => SchemeType::Monochrome,
            "neutral" => SchemeType::Neutral,
            "rainbow" => SchemeType::Rainbow,
            "tonal-spot" => SchemeType::TonalSpot,
            "vibrant" => SchemeType::Vibrant,
            "smart" => SchemeType::Smart,
            _ => return None,
        })
    }

    pub fn variant(self) -> Variant {
        match self {
            SchemeType::Content => Variant::Content,
            SchemeType::Expressive => Variant::Expressive,
            SchemeType::Fidelity => Variant::Fidelity,
            SchemeType::FruitSalad => Variant::FruitSalad,
            SchemeType::Monochrome => Variant::Monochrome,
            SchemeType::Neutral => Variant::Neutral,
            SchemeType::Rainbow => Variant::Rainbow,
            SchemeType::TonalSpot | SchemeType::Smart => Variant::TonalSpot,
            SchemeType::Vibrant => Variant::Vibrant,
        }
    }
}

pub fn ranked_source_colors(pixels: &[Argb]) -> Vec<Argb> {
    let opaque: Vec<Argb> = pixels.iter().copied().filter(|p| p.alpha == 255).collect();
    let mut quantized = QuantizerCelebi::quantize(&opaque, MAX_COLORS);
    quantized.color_to_count.retain(|&argb, _| Cam16::from(argb).chroma >= MIN_CHROMA);
    Score::score(&quantized.color_to_count, None, None, None)
}

pub fn scheme(source: Argb, kind: SchemeType, dark: bool, contrast: Option<f64>) -> Scheme {
    Scheme::from(DynamicScheme::by_variant(source, &kind.variant(), dark, contrast))
}

pub fn roles(scheme: Scheme) -> Vec<(String, Argb)> {
    scheme.into_iter().collect()
}

pub fn hex(color: Argb) -> String {
    format!("#{:02x}{:02x}{:02x}", color.red, color.green, color.blue)
}

pub fn colorfulness(rgb: &[[u8; 3]]) -> f64 {
    if rgb.is_empty() {
        return 0.0;
    }
    let (mut rg_sum, mut yb_sum, mut rg_sq_sum, mut yb_sq_sum) = (0.0f64, 0.0f64, 0.0f64, 0.0f64);
    for pixel in rgb {
        let (r, g, b) = (f64::from(pixel[0]), f64::from(pixel[1]), f64::from(pixel[2]));
        let rg = (r - g).abs();
        let yb = (0.5 * (r + g) - b).abs();
        rg_sum += rg;
        yb_sum += yb;
        rg_sq_sum += rg * rg;
        yb_sq_sum += yb * yb;
    }
    let count = rgb.len() as f64;
    let mean_rg = rg_sum / count;
    let mean_yb = yb_sum / count;
    let variance_rg = (rg_sq_sum / count) - (mean_rg * mean_rg);
    let variance_yb = (yb_sq_sum / count) - (mean_yb * mean_yb);
    let std_rg = variance_rg.sqrt().max(0.0);
    let std_yb = variance_yb.sqrt().max(0.0);
    (std_rg.powi(2) + std_yb.powi(2)).sqrt() + 0.3 * (mean_rg.powi(2) + mean_yb.powi(2)).sqrt()
}

pub fn detect_variant(colorfulness: f64) -> SchemeType {
    match colorfulness {
        ..6.0 => SchemeType::Monochrome,
        6.0..20.0 => SchemeType::Neutral,
        20.0..70.0 => SchemeType::TonalSpot,
        _ => SchemeType::Vibrant,
    }
}

pub fn is_light_average(rgb: [u8; 3]) -> bool {
    Hct::new(Argb::new(255, rgb[0], rgb[1], rgb[2])).get_tone() > 60.0
}

#[cfg(test)]
mod tests {
    use super::*;

    fn find<'a>(roles: &'a [(String, Argb)], name: &str) -> &'a Argb {
        &roles.iter().find(|(n, _)| n == name).unwrap().1
    }

    #[test]
    fn red_tonal_spot_matches_matugens_own_unit_test() {
        let roles = roles(scheme(Argb::new(255, 255, 0, 0), SchemeType::TonalSpot, true, None));
        assert_eq!(*find(&roles, "primary"), Argb::new(255, 255, 180, 168));
        assert_eq!(hex(*find(&roles, "primary")), "#ffb4a8");
        assert_eq!(roles.len(), 49);
    }

    #[test]
    fn scheme_type_names_parse_with_or_without_prefix() {
        assert_eq!(SchemeType::parse("scheme-tonal-spot"), Some(SchemeType::TonalSpot));
        assert_eq!(SchemeType::parse("Fruit-Salad"), Some(SchemeType::FruitSalad));
        assert_eq!(SchemeType::parse("scheme-smart"), Some(SchemeType::Smart));
        assert_eq!(SchemeType::parse("scheme-nope"), None);
        assert_eq!(SchemeType::Smart.variant(), Variant::TonalSpot);
    }

    #[test]
    fn smart_detection_thresholds_follow_matugen() {
        assert_eq!(detect_variant(0.0), SchemeType::Monochrome);
        assert_eq!(detect_variant(6.0), SchemeType::Neutral);
        assert_eq!(detect_variant(20.0), SchemeType::TonalSpot);
        assert_eq!(detect_variant(70.0), SchemeType::Vibrant);
        assert!(colorfulness(&[[128, 128, 128]; 64]) < 1.0);
        assert!(!is_light_average([20, 20, 30]));
        assert!(is_light_average([240, 240, 250]));
    }

    #[test]
    fn solid_colour_images_rank_that_colour_first() {
        let pixels = vec![Argb::new(255, 200, 30, 60); 112 * 112];
        let ranked = ranked_source_colors(&pixels);
        assert_eq!(ranked.first().map(|c| hex(*c)), Some("#c81e3c".to_owned()));
    }

    #[test]
    fn transparent_and_grey_images_fall_back_to_the_default_blue() {
        assert_eq!(hex(ranked_source_colors(&vec![Argb::new(255, 128, 128, 128); 100])[0]), "#4285f4");
        assert_eq!(hex(ranked_source_colors(&vec![Argb::new(0, 255, 0, 0); 100])[0]), "#4285f4");
    }
}
