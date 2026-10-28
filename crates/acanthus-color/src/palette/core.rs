use super::TonalPalette;
use crate::{color::Argb, hct::Cam16};
use core::fmt;

/// The six tonal palettes of a legacy Material 3 scheme.
#[derive(Debug, Hash, PartialEq, Eq)]
pub struct CorePalette {
    /// Primary accent palette.
    pub primary: TonalPalette,
    /// Secondary accent palette.
    pub secondary: TonalPalette,
    /// Tertiary accent palette.
    pub tertiary: TonalPalette,
    /// Neutral palette.
    pub neutral: TonalPalette,
    /// Neutral variant palette.
    pub neutral_variant: TonalPalette,
    /// Error palette.
    pub error: TonalPalette,
}

impl CorePalette {
    /// Number of accent palettes derived from a source colour.
    pub const SIZE: usize = 5;

    /// Builds the palettes from explicit hue and chroma values.
    pub fn new(
        primary: TonalPalette,
        secondary: TonalPalette,
        tertiary: TonalPalette,
        neutral: TonalPalette,
        neutral_variant: TonalPalette,
        error: Option<TonalPalette>,
    ) -> Self {
        Self {
            primary,
            secondary,
            tertiary,
            neutral,
            neutral_variant,
            error: error.unwrap_or_else(|| TonalPalette::of(25.0, 84.0)),
        }
    }

    /// Palettes for the standard scheme derived from `argb`.
    pub fn of(argb: Argb) -> Self {
        let cam = Cam16::from(argb);

        Self::_of(cam.hue, cam.chroma)
    }

    fn _of(hue: f64, chroma: f64) -> Self {
        Self::new(
            TonalPalette::of(hue, 48.0_f64.max(chroma)),
            TonalPalette::of(hue, 16.0),
            TonalPalette::of(hue + 60.0, 24.0),
            TonalPalette::of(hue, 4.0),
            TonalPalette::of(hue, 8.0),
            None,
        )
    }

    /// Palettes for the content scheme derived from `argb`, keeping the source chroma.
    pub fn content_of(argb: Argb) -> Self {
        let cam = Cam16::from(argb);

        Self::_content_of(cam.hue, cam.chroma)
    }

    fn _content_of(hue: f64, chroma: f64) -> Self {
        Self::new(
            TonalPalette::of(hue, chroma),
            TonalPalette::of(hue, chroma / 3.0),
            TonalPalette::of(hue + 60.0, chroma / 2.0),
            TonalPalette::of(hue, (chroma / 12.0).min(4.0)),
            TonalPalette::of(hue, (chroma / 6.0).min(8.0)),
            None,
        )
    }
}

impl fmt::Display for CorePalette {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "primary{} secondary{} tertiary{} neutral{} neutralVariant{}",
            self.primary, self.secondary, self.tertiary, self.neutral, self.neutral_variant
        )
    }
}

fn _get_partition(list: &[i32], partition_number: usize, partition_size: usize) -> &[i32] {
    &list[(partition_number * partition_size)..((partition_number + 1) * partition_size)]
}
