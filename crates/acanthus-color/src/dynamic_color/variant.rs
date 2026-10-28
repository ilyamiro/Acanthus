/// Material 3 scheme variant.
#[derive(Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Clone)]
pub enum Variant {
    /// Grayscale.
    Monochrome,
    /// Almost grayscale with a hint of the source hue.
    Neutral,
    /// Default Material 3 look: calm, medium chroma.
    TonalSpot,
    /// High chroma accents.
    Vibrant,
    /// Playful scheme with hue-shifted accents.
    Expressive,
    /// Accents stay close to the source colour.
    Fidelity,
    /// Like fidelity, with the source colour as the container tone.
    Content,
    /// Playful scheme that keeps the source hue for the primary palette.
    Rainbow,
    /// Bold scheme with a hue-shifted tertiary palette.
    FruitSalad,
}
