use super::{MaterialDynamicColors, Variant};
use crate::{
    color::Argb,
    hct::Hct,
    palette::TonalPalette,
    scheme::variant::{
        SchemeContent, SchemeExpressive, SchemeFidelity, SchemeFruitSalad, SchemeMonochrome, SchemeNeutral,
        SchemeRainbow, SchemeTonalSpot, SchemeVibrant,
    },
    utils::math::sanitize_degrees_double,
};
use core::{
    fmt,
    hash::{Hash, Hasher},
};

/// A Material 3 scheme: the source colour, variant, polarity, contrast and six tonal palettes from which every role resolves.
#[derive(Clone)]
pub struct DynamicScheme {
    /// Source colour.
    pub source_color_argb: Argb,

    /// Source colour as HCT.
    pub source_color_hct: Hct,

    /// Variant of the scheme.
    pub variant: Variant,

    /// Whether the scheme is dark.
    pub is_dark: bool,

    /// Contrast level from -1.0 (reduced) to 1.0 (maximum).
    pub contrast_level: f64,

    /// Primary palette.
    pub primary_palette: TonalPalette,

    /// Secondary palette.
    pub secondary_palette: TonalPalette,

    /// Tertiary palette.
    pub tertiary_palette: TonalPalette,

    /// Neutral palette.
    pub neutral_palette: TonalPalette,

    /// Neutral variant palette.
    pub neutral_variant_palette: TonalPalette,

    /// Error palette.
    pub error_palette: TonalPalette,
}

impl DynamicScheme {
    /// Creates a scheme from explicit palettes.
    pub fn new(
        source_color_argb: Argb,
        source_color_hct: Option<Hct>,
        variant: Variant,
        is_dark: bool,
        contrast_level: Option<f64>,
        primary_palette: TonalPalette,
        secondary_palette: TonalPalette,
        tertiary_palette: TonalPalette,
        neutral_palette: TonalPalette,
        neutral_variant_palette: TonalPalette,
        error_palette: Option<TonalPalette>,
    ) -> Self {
        Self {
            source_color_argb,
            source_color_hct: source_color_hct.unwrap_or_else(|| source_color_argb.into()),
            variant,
            is_dark,
            contrast_level: contrast_level.unwrap_or(0.0),
            primary_palette,
            secondary_palette,
            tertiary_palette,
            neutral_palette,
            neutral_variant_palette,
            error_palette: error_palette.unwrap_or_else(|| TonalPalette::of(25.0, 84.0)),
        }
    }

    /// Scheme of `variant` for `source`; `contrast_level` defaults to 0.0.
    pub fn by_variant(source: Argb, variant: &Variant, is_dark: bool, contrast_level: Option<f64>) -> Self {
        let source_hct = source.into();

        match variant {
            Variant::Monochrome => SchemeMonochrome::new(source_hct, is_dark, contrast_level).scheme,
            Variant::Neutral => SchemeNeutral::new(source_hct, is_dark, contrast_level).scheme,
            Variant::TonalSpot => SchemeTonalSpot::new(source_hct, is_dark, contrast_level).scheme,
            Variant::Vibrant => SchemeVibrant::new(source_hct, is_dark, contrast_level).scheme,
            Variant::Expressive => SchemeExpressive::new(source_hct, is_dark, contrast_level).scheme,
            Variant::Fidelity => SchemeFidelity::new(source_hct, is_dark, contrast_level).scheme,
            Variant::Content => SchemeContent::new(source_hct, is_dark, contrast_level).scheme,
            Variant::Rainbow => SchemeRainbow::new(source_hct, is_dark, contrast_level).scheme,
            Variant::FruitSalad => SchemeFruitSalad::new(source_hct, is_dark, contrast_level).scheme,
        }
    }

    /// Hue rotated by the rotation of the `hues` range that contains `source_hue`.
    pub fn get_rotated_hue(source_hue: f64, hues: &[f64], rotations: &[f64]) -> f64 {
        assert!(hues.len() == rotations.len());

        if rotations.len() == 1 {
            return sanitize_degrees_double(source_hue + rotations[0]);
        }

        if hues.is_empty() || rotations.is_empty() {
            return source_hue;
        }

        let size = hues.len();
        let mut i = 0;

        while i <= (size - 2) {
            let this_hue = hues[i];
            let next_hue = hues[i + 1];

            if this_hue < source_hue && source_hue < next_hue {
                return sanitize_degrees_double(source_hue + rotations[i]);
            }

            i += 1;
        }

        source_hue
    }

    /// Key colour of the primary tonal palette.
    pub fn primary_palette_key_color(&self) -> Argb {
        MaterialDynamicColors::primary_palette_key_color().get_argb(self)
    }
    /// Key colour of the secondary tonal palette.
    pub fn secondary_palette_key_color(&self) -> Argb {
        MaterialDynamicColors::secondary_palette_key_color().get_argb(self)
    }
    /// Key colour of the tertiary tonal palette.
    pub fn tertiary_palette_key_color(&self) -> Argb {
        MaterialDynamicColors::tertiary_palette_key_color().get_argb(self)
    }
    /// Key colour of the neutral tonal palette.
    pub fn neutral_palette_key_color(&self) -> Argb {
        MaterialDynamicColors::neutral_palette_key_color().get_argb(self)
    }
    /// Key colour of the neutral variant tonal palette.
    pub fn neutral_variant_palette_key_color(&self) -> Argb {
        MaterialDynamicColors::neutral_palette_key_color().get_argb(self)
    }
    /// Background colour behind scrollable content.
    pub fn background(&self) -> Argb {
        MaterialDynamicColors::background().get_argb(self)
    }
    /// Content colour drawn on `background`.
    pub fn on_background(&self) -> Argb {
        MaterialDynamicColors::on_background().get_argb(self)
    }
    /// Default colour of component surfaces.
    pub fn surface(&self) -> Argb {
        MaterialDynamicColors::surface().get_argb(self)
    }
    /// Surface colour that stays dim in light and dark schemes.
    pub fn surface_dim(&self) -> Argb {
        MaterialDynamicColors::surface_dim().get_argb(self)
    }
    /// Surface colour that stays bright in light and dark schemes.
    pub fn surface_bright(&self) -> Argb {
        MaterialDynamicColors::surface_bright().get_argb(self)
    }
    /// Surface container with the lowest emphasis.
    pub fn surface_container_lowest(&self) -> Argb {
        MaterialDynamicColors::surface_container_lowest().get_argb(self)
    }
    /// Surface container with low emphasis.
    pub fn surface_container_low(&self) -> Argb {
        MaterialDynamicColors::surface_container_low().get_argb(self)
    }
    /// Surface container with default emphasis.
    pub fn surface_container(&self) -> Argb {
        MaterialDynamicColors::surface_container().get_argb(self)
    }
    /// Surface container with high emphasis.
    pub fn surface_container_high(&self) -> Argb {
        MaterialDynamicColors::surface_container_high().get_argb(self)
    }
    /// Surface container with the highest emphasis.
    pub fn surface_container_highest(&self) -> Argb {
        MaterialDynamicColors::surface_container_highest().get_argb(self)
    }
    /// Content colour drawn on `surface`.
    pub fn on_surface(&self) -> Argb {
        MaterialDynamicColors::on_surface().get_argb(self)
    }
    /// Alternative surface colour for containers and dividers.
    pub fn surface_variant(&self) -> Argb {
        MaterialDynamicColors::surface_variant().get_argb(self)
    }
    /// Lower-emphasis content colour drawn on `surface_variant`.
    pub fn on_surface_variant(&self) -> Argb {
        MaterialDynamicColors::on_surface_variant().get_argb(self)
    }
    /// Surface of inverted polarity, used for snack bars and tooltips.
    pub fn inverse_surface(&self) -> Argb {
        MaterialDynamicColors::inverse_surface().get_argb(self)
    }
    /// Content colour drawn on `inverse_surface`.
    pub fn inverse_on_surface(&self) -> Argb {
        MaterialDynamicColors::inverse_on_surface().get_argb(self)
    }
    /// Outline colour for borders and dividers that need contrast.
    pub fn outline(&self) -> Argb {
        MaterialDynamicColors::outline().get_argb(self)
    }
    /// Decorative outline colour with lower contrast.
    pub fn outline_variant(&self) -> Argb {
        MaterialDynamicColors::outline_variant().get_argb(self)
    }
    /// Colour of drop shadows.
    pub fn shadow(&self) -> Argb {
        MaterialDynamicColors::shadow().get_argb(self)
    }
    /// Colour of the scrim behind modal surfaces.
    pub fn scrim(&self) -> Argb {
        MaterialDynamicColors::scrim().get_argb(self)
    }
    /// Tint overlaid on elevated surfaces.
    pub fn surface_tint(&self) -> Argb {
        MaterialDynamicColors::surface_tint().get_argb(self)
    }
    /// Primary accent colour for prominent components.
    pub fn primary(&self) -> Argb {
        MaterialDynamicColors::primary().get_argb(self)
    }
    /// Content colour drawn on `primary`.
    pub fn on_primary(&self) -> Argb {
        MaterialDynamicColors::on_primary().get_argb(self)
    }
    /// Fill colour of primary containers.
    pub fn primary_container(&self) -> Argb {
        MaterialDynamicColors::primary_container().get_argb(self)
    }
    /// Content colour drawn on `primary_container`.
    pub fn on_primary_container(&self) -> Argb {
        MaterialDynamicColors::on_primary_container().get_argb(self)
    }
    /// Primary accent drawn on `inverse_surface`.
    pub fn inverse_primary(&self) -> Argb {
        MaterialDynamicColors::inverse_primary().get_argb(self)
    }
    /// Secondary accent colour for less prominent components.
    pub fn secondary(&self) -> Argb {
        MaterialDynamicColors::secondary().get_argb(self)
    }
    /// Content colour drawn on `secondary`.
    pub fn on_secondary(&self) -> Argb {
        MaterialDynamicColors::on_secondary().get_argb(self)
    }
    /// Fill colour of secondary containers.
    pub fn secondary_container(&self) -> Argb {
        MaterialDynamicColors::secondary_container().get_argb(self)
    }
    /// Content colour drawn on `secondary_container`.
    pub fn on_secondary_container(&self) -> Argb {
        MaterialDynamicColors::on_secondary_container().get_argb(self)
    }
    /// Tertiary accent colour that balances primary and secondary.
    pub fn tertiary(&self) -> Argb {
        MaterialDynamicColors::tertiary().get_argb(self)
    }
    /// Content colour drawn on `tertiary`.
    pub fn on_tertiary(&self) -> Argb {
        MaterialDynamicColors::on_tertiary().get_argb(self)
    }
    /// Fill colour of tertiary containers.
    pub fn tertiary_container(&self) -> Argb {
        MaterialDynamicColors::tertiary_container().get_argb(self)
    }
    /// Content colour drawn on `tertiary_container`.
    pub fn on_tertiary_container(&self) -> Argb {
        MaterialDynamicColors::on_tertiary_container().get_argb(self)
    }
    /// Colour of error states.
    pub fn error(&self) -> Argb {
        MaterialDynamicColors::error().get_argb(self)
    }
    /// Content colour drawn on `error`.
    pub fn on_error(&self) -> Argb {
        MaterialDynamicColors::on_error().get_argb(self)
    }
    /// Fill colour of error containers.
    pub fn error_container(&self) -> Argb {
        MaterialDynamicColors::error_container().get_argb(self)
    }
    /// Content colour drawn on `error_container`.
    pub fn on_error_container(&self) -> Argb {
        MaterialDynamicColors::on_error_container().get_argb(self)
    }
    /// Primary accent that keeps its tone in light and dark schemes.
    pub fn primary_fixed(&self) -> Argb {
        MaterialDynamicColors::primary_fixed().get_argb(self)
    }
    /// Dimmer variant of `primary_fixed`.
    pub fn primary_fixed_dim(&self) -> Argb {
        MaterialDynamicColors::primary_fixed_dim().get_argb(self)
    }
    /// Content colour drawn on `primary_fixed`.
    pub fn on_primary_fixed(&self) -> Argb {
        MaterialDynamicColors::on_primary_fixed().get_argb(self)
    }
    /// Lower-emphasis content colour drawn on `primary_fixed`.
    pub fn on_primary_fixed_variant(&self) -> Argb {
        MaterialDynamicColors::on_primary_fixed_variant().get_argb(self)
    }
    /// Secondary accent that keeps its tone in light and dark schemes.
    pub fn secondary_fixed(&self) -> Argb {
        MaterialDynamicColors::secondary_fixed().get_argb(self)
    }
    /// Dimmer variant of `secondary_fixed`.
    pub fn secondary_fixed_dim(&self) -> Argb {
        MaterialDynamicColors::secondary_fixed_dim().get_argb(self)
    }
    /// Content colour drawn on `secondary_fixed`.
    pub fn on_secondary_fixed(&self) -> Argb {
        MaterialDynamicColors::on_secondary_fixed().get_argb(self)
    }
    /// Lower-emphasis content colour drawn on `secondary_fixed`.
    pub fn on_secondary_fixed_variant(&self) -> Argb {
        MaterialDynamicColors::on_secondary_fixed_variant().get_argb(self)
    }
    /// Tertiary accent that keeps its tone in light and dark schemes.
    pub fn tertiary_fixed(&self) -> Argb {
        MaterialDynamicColors::tertiary_fixed().get_argb(self)
    }
    /// Dimmer variant of `tertiary_fixed`.
    pub fn tertiary_fixed_dim(&self) -> Argb {
        MaterialDynamicColors::tertiary_fixed_dim().get_argb(self)
    }
    /// Content colour drawn on `tertiary_fixed`.
    pub fn on_tertiary_fixed(&self) -> Argb {
        MaterialDynamicColors::on_tertiary_fixed().get_argb(self)
    }
    /// Lower-emphasis content colour drawn on `tertiary_fixed`.
    pub fn on_tertiary_fixed_variant(&self) -> Argb {
        MaterialDynamicColors::on_tertiary_fixed_variant().get_argb(self)
    }
}

impl PartialEq for DynamicScheme {
    fn eq(&self, other: &Self) -> bool {
        self.source_color_argb == other.source_color_argb
            && self.source_color_hct == other.source_color_hct
            && self.variant == other.variant
            && self.is_dark == other.is_dark
            && self.contrast_level == other.contrast_level
            && self.primary_palette == other.primary_palette
            && self.secondary_palette == other.secondary_palette
            && self.tertiary_palette == other.tertiary_palette
            && self.neutral_palette == other.neutral_palette
            && self.neutral_variant_palette == other.neutral_variant_palette
            && self.error_palette == other.error_palette
    }
}

impl Eq for DynamicScheme {}

impl Hash for DynamicScheme {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.source_color_argb.hash(state);
        self.source_color_hct.hash(state);
        self.variant.hash(state);
        self.is_dark.hash(state);
        self.contrast_level.to_bits().hash(state);
        self.primary_palette.hash(state);
        self.secondary_palette.hash(state);
        self.tertiary_palette.hash(state);
        self.neutral_palette.hash(state);
        self.neutral_variant_palette.hash(state);
        self.error_palette.hash(state);
    }
}

impl fmt::Display for DynamicScheme {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "Scheme {{")?;
        writeln!(f, "  primary = {}", self.primary())?;
        writeln!(f, "  on_primary = {}", self.on_primary())?;
        writeln!(f, "  primary_container = {}", self.primary_container())?;
        writeln!(f, "  on_primary_container = {}", self.on_primary_container())?;
        writeln!(f, "  secondary = {}", self.secondary())?;
        writeln!(f, "  on_secondary = {}", self.on_secondary())?;
        writeln!(f, "  secondary_container = {}", self.secondary_container())?;
        writeln!(f, "  on_secondary_container = {}", self.on_secondary_container())?;
        writeln!(f, "  tertiary = {}", self.tertiary())?;
        writeln!(f, "  on_tertiary = {}", self.on_tertiary())?;
        writeln!(f, "  tertiary_container = {}", self.tertiary_container())?;
        writeln!(f, "  on_tertiary_container = {}", self.on_tertiary_container())?;
        writeln!(f, "  error = {}", self.error())?;
        writeln!(f, "  on_error = {}", self.on_error())?;
        writeln!(f, "  error_container = {}", self.error_container())?;
        writeln!(f, "  on_error_container = {}", self.on_error_container())?;
        writeln!(f, "  background = {}", self.background())?;
        writeln!(f, "  on_background = {}", self.on_background())?;
        writeln!(f, "  surface = {}", self.surface())?;
        writeln!(f, "  on_surface = {}", self.on_surface())?;
        writeln!(f, "  surface_variant = {}", self.surface_variant())?;
        writeln!(f, "  on_surface_variant = {}", self.on_surface_variant())?;
        writeln!(f, "  outline = {}", self.outline())?;
        writeln!(f, "  outline_variant = {}", self.outline_variant())?;
        writeln!(f, "  shadow = {}", self.shadow())?;
        writeln!(f, "  scrim = {}", self.scrim())?;
        writeln!(f, "  inverse_surface = {}", self.inverse_surface())?;
        writeln!(f, "  inverse_on_surface = {}", self.inverse_on_surface())?;
        writeln!(f, "  inverse_primary = {}", self.inverse_primary())?;
        writeln!(f, "}}")
    }
}
