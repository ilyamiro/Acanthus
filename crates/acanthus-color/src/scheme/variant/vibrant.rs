use crate::{
    dynamic_color::{DynamicScheme, Variant},
    hct::Hct,
    palette::{Palette, TonalPalette},
};

/// Material 3 vibrant scheme.
pub struct SchemeVibrant {
    /// The generated scheme.
    pub scheme: DynamicScheme,
}

impl SchemeVibrant {
    /// Source hue anchors for the rotations.
    pub const HUES: [f64; 9] = [0.0, 41.0, 61.0, 101.0, 131.0, 181.0, 251.0, 301.0, 360.0];

    /// Hue rotations of the secondary palette, one per hue anchor.
    pub const SECONDARY_ROTATIONS: [f64; 9] = [18.0, 15.0, 10.0, 12.0, 15.0, 18.0, 15.0, 12.0, 12.0];

    /// Hue rotations of the tertiary palette, one per hue anchor.
    pub const TERTIARY_ROTATIONS: [f64; 9] = [35.0, 30.0, 20.0, 25.0, 30.0, 35.0, 30.0, 25.0, 25.0];

    /// Scheme for a source colour, polarity and optional contrast level (default 0.0).
    pub fn new(source_color_hct: Hct, is_dark: bool, contrast_level: Option<f64>) -> Self {
        Self {
            scheme: DynamicScheme::new(
                source_color_hct.into(),
                None,
                Variant::Vibrant,
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
            Palette::Primary => TonalPalette::of(source_color_hct.get_hue(), 200.0),
            Palette::Secondary => TonalPalette::of(
                DynamicScheme::get_rotated_hue(source_color_hct.get_hue(), &Self::HUES, &Self::SECONDARY_ROTATIONS),
                24.0,
            ),
            Palette::Tertiary => TonalPalette::of(
                DynamicScheme::get_rotated_hue(source_color_hct.get_hue(), &Self::HUES, &Self::TERTIARY_ROTATIONS),
                32.0,
            ),
            Palette::Error => TonalPalette::of(25.0, 84.0),
            Palette::Neutral | Palette::NeutralVariant => TonalPalette::of(source_color_hct.get_hue(), 10.0),
        }
    }
}
