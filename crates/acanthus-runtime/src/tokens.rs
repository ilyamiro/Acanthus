/// Durations of the standard transitions, in milliseconds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MotionTokens {
    /// Short transitions: colour changes on hover.
    pub fast: u32,
    /// Default transitions: scale, rotation, appearance.
    pub base: u32,
    /// Long transitions: settling after a press.
    pub slow: u32,
}

impl Default for MotionTokens {
    fn default() -> Self {
        MotionTokens { fast: acanthus_anim::FAST, base: acanthus_anim::BASE, slow: acanthus_anim::SLOW }
    }
}

/// The spacing scale, in logical pixels.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SpacingTokens {
    /// Tightest gap.
    pub xs: f32,
    /// Small gap.
    pub sm: f32,
    /// Default gap.
    pub md: f32,
    /// Large gap.
    pub lg: f32,
    /// Section gap.
    pub xl: f32,
}

impl Default for SpacingTokens {
    fn default() -> Self {
        SpacingTokens { xs: 4.0, sm: 8.0, md: 12.0, lg: 16.0, xl: 24.0 }
    }
}

/// The corner radius scale, in logical pixels.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RadiusTokens {
    /// Small corners: chips, handles.
    pub sm: f32,
    /// Default corners: buttons, fields.
    pub md: f32,
    /// Large corners: cards, popups.
    pub lg: f32,
    /// Fully round ends.
    pub pill: f32,
}

impl Default for RadiusTokens {
    fn default() -> Self {
        RadiusTokens { sm: 4.0, md: 8.0, lg: 16.0, pill: 999.0 }
    }
}

/// The fixed design scales of a theme: motion durations, spacing and radii. An application replaces the defaults before building its views.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Tokens {
    /// Transition durations.
    pub motion: MotionTokens,
    /// Spacing scale.
    pub spacing: SpacingTokens,
    /// Radius scale.
    pub radii: RadiusTokens,
}

impl Tokens {
    /// The spacing or radius entry named `name` (`spacing.xs` to `spacing.xl`, `radii.sm` to `radii.pill`); `None` for any other name.
    pub fn scale(&self, name: &str) -> Option<f32> {
        let (group, step) = name.split_once('.')?;
        match (group, step) {
            ("spacing", "xs") => Some(self.spacing.xs),
            ("spacing", "sm") => Some(self.spacing.sm),
            ("spacing", "md") => Some(self.spacing.md),
            ("spacing", "lg") => Some(self.spacing.lg),
            ("spacing", "xl") => Some(self.spacing.xl),
            ("radii", "sm") => Some(self.radii.sm),
            ("radii", "md") => Some(self.radii.md),
            ("radii", "lg") => Some(self.radii.lg),
            ("radii", "pill") => Some(self.radii.pill),
            _ => None,
        }
    }
}

#[cfg(test)]
mod scale_tests {
    use super::*;

    #[test]
    fn scale_names_resolve_to_the_token_values() {
        let tokens = Tokens::default();
        assert_eq!(tokens.scale("spacing.md"), Some(12.0));
        assert_eq!(tokens.scale("radii.pill"), Some(999.0));
        assert_eq!(tokens.scale("radii.xl"), None);
        assert_eq!(tokens.scale("motion.fast"), None);
        assert_eq!(tokens.scale("spacing"), None);
    }
}
