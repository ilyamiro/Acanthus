use super::TonalPalette;
use crate::{color::Argb, hct::Cam16};
use core::fmt;

#[derive(Debug, Hash, PartialEq, Eq)]
pub struct CorePalette {
    pub primary: TonalPalette,
    pub secondary: TonalPalette,
    pub tertiary: TonalPalette,
    pub neutral: TonalPalette,
    pub neutral_variant: TonalPalette,
    pub error: TonalPalette,
}

impl CorePalette {
    pub const SIZE: usize = 5;

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
