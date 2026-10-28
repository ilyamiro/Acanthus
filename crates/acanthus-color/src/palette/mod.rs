pub use self::core::CorePalette;
pub use tonal::TonalPalette;

mod core;
mod tonal;

/// Role of a tonal palette inside a scheme.
#[derive(PartialEq, Eq)]
pub enum Palette {
    /// Primary accent palette.
    Primary,
    /// Secondary accent palette.
    Secondary,
    /// Tertiary accent palette.
    Tertiary,
    /// Error palette.
    Error,
    /// Neutral palette for surfaces.
    Neutral,
    /// Neutral variant palette for outlines and variants.
    NeutralVariant,
}
