use super::Palette;
use crate::{
    color::Argb,
    dynamic_color::Variant,
    hct::Hct,
    scheme::variant::{
        SchemeContent, SchemeExpressive, SchemeFidelity, SchemeFruitSalad, SchemeMonochrome, SchemeNeutral,
        SchemeRainbow, SchemeTonalSpot, SchemeVibrant,
    },
};
use core::{
    fmt,
    hash::{Hash, Hasher},
};

#[derive(Clone, Copy, Debug)]
pub struct TonalPalette {
    _hue: f64,
    _chroma: f64,
    _key_color: Hct,
}

impl TonalPalette {
    const COMMON_TONES: [i32; 13] = [0, 10, 20, 30, 40, 50, 60, 70, 80, 90, 95, 99, 100];

    pub const fn common_size() -> usize {
        Self::COMMON_TONES.len()
    }

    pub const fn hue(&self) -> f64 {
        self._hue
    }

    pub const fn chroma(&self) -> f64 {
        self._chroma
    }

    pub const fn key_color(&self) -> Hct {
        self._key_color
    }

    const fn new(_hue: f64, _chroma: f64, _key_color: Hct) -> Self {
        Self { _hue, _chroma, _key_color }
    }

    pub const fn from_hct(hct: Hct) -> Self {
        Self::new(hct.get_hue(), hct.get_chroma(), hct)
    }

    pub fn by_variant(source_hct: &Hct, scheme: &Variant, variant: &Palette) -> Self {
        match scheme {
            Variant::Monochrome => SchemeMonochrome::palette(source_hct, variant),
            Variant::Neutral => SchemeNeutral::palette(source_hct, variant),
            Variant::TonalSpot => SchemeTonalSpot::palette(source_hct, variant),
            Variant::Vibrant => SchemeVibrant::palette(source_hct, variant),
            Variant::Expressive => SchemeExpressive::palette(source_hct, variant),
            Variant::Fidelity => SchemeFidelity::palette(source_hct, variant),
            Variant::Content => SchemeContent::palette(source_hct, variant),
            Variant::Rainbow => SchemeRainbow::palette(source_hct, variant),
            Variant::FruitSalad => SchemeFruitSalad::palette(source_hct, variant),
        }
    }

    pub fn from_hue_and_chroma(hue: f64, chroma: f64) -> Self {
        Self::new(hue, chroma, Self::create_key_color(hue, chroma))
    }

    pub fn of(hue: f64, chroma: f64) -> Self {
        Self::from_hue_and_chroma(hue, chroma)
    }

    pub fn create_key_color(hue: f64, chroma: f64) -> Hct {
        let start_tone = 50.0;
        let mut smallest_delta_hct = Hct::from(hue, chroma, start_tone);
        let mut smallest_delta = (smallest_delta_hct.get_chroma() - chroma).abs();

        for delta in 1..=49 {
            if (chroma.round() - smallest_delta_hct.get_chroma().round()).abs() < f64::EPSILON {
                return smallest_delta_hct;
            }

            let hct_add = Hct::from(hue, chroma, start_tone + f64::from(delta));
            let hct_add_delta = (hct_add.get_chroma() - chroma).abs();

            if hct_add_delta < smallest_delta {
                smallest_delta = hct_add_delta;
                smallest_delta_hct = hct_add;
            }

            let hct_subtract = Hct::from(hue, chroma, start_tone - f64::from(delta));
            let hct_subtract_delta = (hct_subtract.get_chroma() - chroma).abs();

            if hct_subtract_delta < smallest_delta {
                smallest_delta = hct_subtract_delta;
                smallest_delta_hct = hct_subtract;
            }
        }

        smallest_delta_hct
    }

    pub fn tone(&self, tone: i32) -> Argb {
        Hct::from(self.hue(), self.chroma(), f64::from(tone)).into()
    }

    pub fn get_hct(&self, tone: f64) -> Hct {
        Hct::from(self.hue(), self.chroma(), tone)
    }
}

impl PartialEq for TonalPalette {
    fn eq(&self, other: &Self) -> bool {
        self._hue == other._hue && self._chroma == other._chroma
    }
}

impl Eq for TonalPalette {}

impl Hash for TonalPalette {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self._hue.to_bits().hash(state);
        self._chroma.to_bits().hash(state);
        self._key_color.hash(state);
    }
}

impl fmt::Display for TonalPalette {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "TonalPalette.of({}, {})", self.hue(), self.chroma())
    }
}
