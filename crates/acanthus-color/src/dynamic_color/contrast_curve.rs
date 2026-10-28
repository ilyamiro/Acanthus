use crate::utils::math::lerp;

/// Contrast ratio targets for the four contrast levels.
pub struct ContrastCurve {
    /// Target at contrast level -1.0.
    pub low: f64,
    /// Target at contrast level 0.0.
    pub normal: f64,
    /// Target at contrast level 0.5.
    pub medium: f64,
    /// Target at contrast level 1.0.
    pub high: f64,
}

impl ContrastCurve {
    /// Target for `contrast_level`, interpolated between the four anchors.
    pub fn get(&self, contrast_level: f64) -> f64 {
        match contrast_level {
            contrast_level if contrast_level <= -1.0 => self.low,
            contrast_level if contrast_level < 0.0 => lerp(self.low, self.normal, (contrast_level - (-1.0)) / 1.0),
            contrast_level if contrast_level < 0.5 => lerp(self.normal, self.medium, (contrast_level - 0.0) / 0.5),
            contrast_level if contrast_level < 1.0 => lerp(self.medium, self.high, (contrast_level - 0.5) / 0.5),
            _ => self.high,
        }
    }
}
