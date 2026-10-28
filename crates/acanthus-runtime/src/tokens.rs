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
