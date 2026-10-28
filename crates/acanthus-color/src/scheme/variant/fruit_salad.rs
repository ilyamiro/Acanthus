use crate::{
    dynamic_color::{DynamicScheme, Variant},
    hct::Hct,
    palette::{Palette, TonalPalette},
    utils::math::sanitize_degrees_double,
};

/// Material 3 fruit salad scheme.
pub struct SchemeFruitSalad {
    /// The generated scheme.
    pub scheme: DynamicScheme,
}

impl SchemeFruitSalad {
    /// Scheme for a source colour, polarity and optional contrast level (default 0.0).
    pub fn new(source_color_hct: Hct, is_dark: bool, contrast_level: Option<f64>) -> Self {
        Self {
            scheme: DynamicScheme::new(
                source_color_hct.into(),
                None,
                Variant::FruitSalad,
                is_dark,
                contrast_level,
                Self::palette(&source_color_hct, &Palette::Primary),
                Self::palette(&source_color_hct, &Palette::Secondary),
                Self::palette(&source_color_hct, &Palette::Tertiary),
                Self::palette(&source_color_hct, &Palette::Neutral),
                Self::palette(&source_color_hct, &Palette::NeutralVariant),
                None,
            ),
        }
    }

    /// Tonal palette for `variant` derived from `source_color_hct`.
    pub fn palette(source_color_hct: &Hct, variant: &Palette) -> TonalPalette {
        match variant {
            Palette::Primary => TonalPalette::of(sanitize_degrees_double(source_color_hct.get_hue() - 50.0), 48.0),
            Palette::Secondary => TonalPalette::of(sanitize_degrees_double(source_color_hct.get_hue() - 50.0), 36.0),
            Palette::Tertiary => TonalPalette::of(source_color_hct.get_hue(), 36.0),
            Palette::Error => TonalPalette::of(25.0, 84.0),
            Palette::Neutral => TonalPalette::of(source_color_hct.get_hue(), 10.0),
            Palette::NeutralVariant => TonalPalette::of(source_color_hct.get_hue(), 16.0),
        }
    }
}
