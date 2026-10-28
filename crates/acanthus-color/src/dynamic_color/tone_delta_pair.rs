use super::DynamicColor;

/// Direction of the tone difference between two colours.
#[derive(PartialEq, Eq)]
pub enum TonePolarity {
    /// The subject is darker than the basis.
    Darker,
    /// The subject is lighter than the basis.
    Lighter,
    /// The subject is nearer to the surface tone than the basis.
    Nearer,
    /// The subject is farther from the surface tone than the basis.
    Farther,
}

/// Constraint keeping two colours at least `delta` tones apart.
pub struct ToneDeltaPair {
    /// Colour whose tone is adjusted.
    pub subject: DynamicColor,
    /// Colour the subject is measured against.
    pub basis: DynamicColor,
    /// Minimum tone difference.
    pub delta: f64,
    /// Direction of the difference.
    pub polarity: TonePolarity,
    /// Whether both colours are moved together to keep the difference.
    pub stay_together: bool,
}

impl ToneDeltaPair {
    /// Creates a constraint.
    pub const fn new(
        subject: DynamicColor,
        basis: DynamicColor,
        delta: f64,
        polarity: TonePolarity,
        stay_together: bool,
    ) -> Self {
        Self { subject, basis, delta, polarity, stay_together }
    }
}
