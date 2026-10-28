#![allow(clippy::too_many_arguments)]

use crate::{
    Map,
    color::Argb,
    contrast::{darker, darker_unsafe, lighter, lighter_unsafe, ratio_of_tones},
    hct::Hct,
    palette::TonalPalette,
};

pub use {
    contrast_curve::ContrastCurve, dynamic_scheme::DynamicScheme, material_dynamic_colors::MaterialDynamicColors,
    tone_delta_pair::ToneDeltaPair, tone_delta_pair::TonePolarity, variant::Variant,
};

/// Contrast curves interpolated by contrast level.
pub mod contrast_curve;
/// Scheme that resolves dynamic colours.
pub mod dynamic_scheme;
/// The Material 3 colour roles.
pub mod material_dynamic_colors;
/// Tone separation constraints between two colours.
pub mod tone_delta_pair;
/// Scheme variants.
pub mod variant;

type DynamicSchemeFn<T> = fn(&DynamicScheme) -> T;
type DynamicSchemeFnRef<T> = fn(&DynamicScheme) -> &T;

/// A colour role defined as a function of a [`DynamicScheme`].
pub struct DynamicColor {
    /// Name of the role.
    pub name: String,
    palette: Box<DynamicSchemeFnRef<TonalPalette>>,
    tone: Box<fn(&DynamicScheme) -> f64>,
    is_background: bool,
    background: Option<Box<DynamicSchemeFn<DynamicColor>>>,
    second_background: Option<Box<DynamicSchemeFn<DynamicColor>>>,
    contrast_curve: Option<ContrastCurve>,
    tone_delta_pair: Option<Box<DynamicSchemeFn<ToneDeltaPair>>>,
    _hct_cache: Map<DynamicScheme, Hct>,
}

impl DynamicColor {
    /// Creates a role from its palette, tone and optional background, contrast curve and tone-delta pair.
    pub fn new<T: Into<String>>(
        name: T,
        palette: fn(&DynamicScheme) -> &TonalPalette,
        tone: fn(&DynamicScheme) -> f64,
        is_background: bool,
        background: Option<fn(&DynamicScheme) -> Self>,
        second_background: Option<fn(&DynamicScheme) -> Self>,
        contrast_curve: Option<ContrastCurve>,
        tone_delta_pair: Option<fn(&DynamicScheme) -> ToneDeltaPair>,
    ) -> Self {
        Self {
            name: name.into(),
            palette: Box::new(palette),
            tone: Box::new(tone),
            is_background,
            background: background.map(Box::new),
            second_background: second_background.map(Box::new),
            contrast_curve,
            tone_delta_pair: tone_delta_pair.map(Box::new),
            _hct_cache: Map::default(),
        }
    }

    /// ARGB of the role in `scheme`.
    pub fn get_argb(&mut self, scheme: &DynamicScheme) -> Argb {
        self.get_hct(scheme).into()
    }

    /// HCT of the role in `scheme`, cached per scheme.
    pub fn get_hct(&mut self, scheme: &DynamicScheme) -> Hct {
        if let Some(cached_answer) = self._hct_cache.get(scheme) {
            return *cached_answer;
        }

        let answer = (self.palette)(scheme).get_hct(self.get_tone(scheme));

        if self._hct_cache.len() > 4 {
            self._hct_cache.clear();
        }

        self._hct_cache.insert(scheme.clone(), answer);

        answer
    }

    /// Tone of the role in `scheme` after contrast and tone-delta adjustments.
    pub fn get_tone(&self, scheme: &DynamicScheme) -> f64 {
        let decreasing_contrast = scheme.contrast_level < 0.0;

        if let Some(tone_delta_pair) = &self.tone_delta_pair {
            let pair = (tone_delta_pair)(scheme);
            let role_a = pair.subject;
            let role_b = pair.basis;
            let delta = pair.delta;
            let polarity = pair.polarity;
            let stay_together = pair.stay_together;

            let bg = self.background.as_ref().unwrap()(scheme);
            let bg_tone = bg.get_tone(scheme);

            let a_is_nearer = polarity == TonePolarity::Nearer
                || (polarity == TonePolarity::Lighter && !scheme.is_dark)
                || (polarity == TonePolarity::Darker && scheme.is_dark);
            let nearer = if a_is_nearer { &role_a } else { &role_b };
            let farther = if a_is_nearer { &role_b } else { &role_a };
            let am_nearer = self.name == nearer.name;
            let expansion_dir = if scheme.is_dark { 1.0 } else { -1.0 };

            let n_contrast = nearer.contrast_curve.as_ref().unwrap().get(scheme.contrast_level);
            let f_contrast = farther.contrast_curve.as_ref().unwrap().get(scheme.contrast_level);

            let n_initial_tone = (nearer.tone)(scheme);
            let mut n_tone = if decreasing_contrast {
                Self::foreground_tone(bg_tone, n_contrast)
            } else if ratio_of_tones(bg_tone, n_initial_tone) >= n_contrast {
                n_initial_tone
            } else {
                Self::foreground_tone(bg_tone, n_contrast)
            };
            let f_initial_tone = (farther.tone)(scheme);
            let mut f_tone = if decreasing_contrast {
                Self::foreground_tone(bg_tone, f_contrast)
            } else if ratio_of_tones(bg_tone, f_initial_tone) >= f_contrast {
                f_initial_tone
            } else {
                Self::foreground_tone(bg_tone, f_contrast)
            };

            if (f_tone - n_tone) * expansion_dir >= delta {
            } else {
                f_tone = delta.mul_add(expansion_dir, n_tone).clamp(0.0, 100.0);

                if (f_tone - n_tone) * expansion_dir >= delta {
                } else {
                    n_tone = delta.mul_add(-expansion_dir, f_tone).clamp(0.0, 100.0);
                }
            }

            if (50.0..60.0).contains(&n_tone) {
                if expansion_dir > 0.0 {
                    n_tone = 60.0;

                    f_tone = f_tone.max(delta.mul_add(expansion_dir, n_tone));
                } else {
                    n_tone = 49.0;

                    f_tone = f_tone.min(delta.mul_add(expansion_dir, n_tone));
                }
            } else if (50.0..60.0).contains(&f_tone) {
                if stay_together {
                    if expansion_dir > 0.0 {
                        n_tone = 60.0;

                        f_tone = f_tone.max(delta.mul_add(expansion_dir, n_tone));
                    } else {
                        n_tone = 49.0;

                        f_tone = f_tone.min(delta.mul_add(expansion_dir, n_tone));
                    }
                } else {
                    if expansion_dir > 0.0 {
                        f_tone = 60.0;
                    } else {
                        f_tone = 49.0;
                    }
                }
            }

            if am_nearer { n_tone } else { f_tone }
        } else {
            let mut answer = (self.tone)(scheme);

            if let Some(background) = &self.background {
                let bg_tone = background(scheme).get_tone(scheme);

                let desired_ratio = self.contrast_curve.as_ref().unwrap().get(scheme.contrast_level);

                if ratio_of_tones(bg_tone, answer) >= desired_ratio {
                } else {
                    answer = Self::foreground_tone(bg_tone, desired_ratio);
                }

                if decreasing_contrast {
                    answer = Self::foreground_tone(bg_tone, desired_ratio);
                }

                if self.is_background && (50.0..60.0).contains(&answer) {
                    if ratio_of_tones(49.0, bg_tone) >= desired_ratio {
                        answer = 49.0;
                    } else {
                        answer = 60.0;
                    }
                }

                if let Some(second_background) = &self.second_background {
                    let bg_tone1 = self.background.as_ref().unwrap()(scheme).get_tone(scheme);
                    let bg_tone2 = second_background(scheme).get_tone(scheme);

                    let upper = bg_tone1.max(bg_tone2);
                    let lower = bg_tone1.min(bg_tone2);

                    if ratio_of_tones(upper, answer) >= desired_ratio && ratio_of_tones(lower, answer) >= desired_ratio
                    {
                        return answer;
                    }

                    let light_option = lighter(upper, desired_ratio);

                    let dark_option = darker(lower, desired_ratio);

                    let mut availables: Vec<f64> = vec![];

                    if (light_option - -1.0).abs() > f64::EPSILON {
                        availables.push(light_option);
                    }

                    if (dark_option - -1.0).abs() > f64::EPSILON {
                        availables.push(dark_option);
                    }

                    let prefers_light =
                        Self::tone_prefers_light_foreground(bg_tone1) || Self::tone_prefers_light_foreground(bg_tone2);

                    if prefers_light {
                        return if light_option < 0.0 { 100.0 } else { light_option };
                    }

                    if availables.len() == 1 {
                        return availables[0];
                    }

                    return if dark_option < 0.0 { 0.0 } else { dark_option };
                }
            }

            answer
        }
    }

    /// Tone of a foreground that reaches `ratio` against `bg_tone`, preferring the lighter side when neither works.
    pub fn foreground_tone(bg_tone: f64, ratio: f64) -> f64 {
        let lighter_tone = lighter_unsafe(bg_tone, ratio);
        let darker_tone = darker_unsafe(bg_tone, ratio);
        let lighter_ratio = ratio_of_tones(lighter_tone, bg_tone);
        let darker_ratio = ratio_of_tones(darker_tone, bg_tone);
        let prefer_ligher = Self::tone_prefers_light_foreground(bg_tone);

        if prefer_ligher {
            let negligible_difference =
                (lighter_ratio - darker_ratio).abs() < 0.1 && lighter_ratio < ratio && darker_ratio < ratio;

            if lighter_ratio >= ratio || lighter_ratio >= darker_ratio || negligible_difference {
                lighter_tone
            } else {
                darker_tone
            }
        } else if darker_ratio >= ratio || darker_ratio >= lighter_ratio {
            darker_tone
        } else {
            lighter_tone
        }
    }

    /// Raises tones 50 to 59 to 60 so light foregrounds stay legible; other tones are unchanged.
    pub fn enable_light_foreground(tone: f64) -> f64 {
        if Self::tone_prefers_light_foreground(tone) && !Self::tone_allows_light_foreground(tone) {
            return 49.0;
        }

        tone
    }

    /// Whether a light foreground is preferred on `tone` (rounded tone below 60).
    pub fn tone_prefers_light_foreground(tone: f64) -> bool {
        tone.round() < 60.0
    }

    /// Whether a light foreground still reaches contrast on `tone` (rounded tone of at most 49 or at least 60).
    pub fn tone_allows_light_foreground(tone: f64) -> bool {
        tone.round() <= 49.0
    }
}
