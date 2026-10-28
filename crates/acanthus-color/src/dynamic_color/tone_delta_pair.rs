use super::DynamicColor;

#[derive(PartialEq, Eq)]
pub enum TonePolarity {
    Darker,
    Lighter,
    Nearer,
    Farther,
}

pub struct ToneDeltaPair {
    pub subject: DynamicColor,
    pub basis: DynamicColor,
    pub delta: f64,
    pub polarity: TonePolarity,
    pub stay_together: bool,
}

impl ToneDeltaPair {
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
