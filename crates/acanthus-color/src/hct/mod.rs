use crate::color::{Argb, lstar_from_y};
use core::{
    fmt,
    hash::{Hash, Hasher},
};
pub use {cam16::Cam16, solver::HctSolver, viewing_conditions::ViewingConditions};

/// CAM16 colour appearance model.
pub mod cam16;
/// Solver that finds the sRGB colour for a hue, chroma and tone.
pub mod solver;
/// Viewing conditions for CAM16.
pub mod viewing_conditions;

/// Hue, chroma and tone: a colour space whose tone is L* and whose hue and chroma follow CAM16, clamped to sRGB.
#[derive(Clone, Copy, Debug)]
pub struct Hct {
    _hue: f64,
    _chroma: f64,
    _tone: f64,
    _argb: Argb,
}

impl Hct {
    /// Hue in degrees.
    pub const fn get_hue(&self) -> f64 {
        self._hue
    }

    /// Sets the hue, keeping chroma and tone as far as sRGB allows.
    pub fn set_hue(&mut self, value: f64) {
        self._argb = HctSolver::solve_to_argb(value, self.get_chroma(), self.get_tone());

        let cam16 = Cam16::from(self._argb);

        self._hue = cam16.hue;
        self._chroma = cam16.chroma;
        self._tone = self._argb.as_lstar();
    }

    /// Chroma.
    pub const fn get_chroma(&self) -> f64 {
        self._chroma
    }

    /// Sets the chroma, keeping hue and tone as far as sRGB allows.
    pub fn set_chroma(&mut self, value: f64) {
        self._argb = HctSolver::solve_to_argb(self.get_hue(), value, self.get_tone());

        let cam16 = Cam16::from(self._argb);

        self._hue = cam16.hue;
        self._chroma = cam16.chroma;
        self._tone = self._argb.as_lstar();
    }

    /// Tone (L*, 0 to 100).
    pub const fn get_tone(&self) -> f64 {
        self._tone
    }

    /// Sets the tone, keeping hue and chroma as far as sRGB allows.
    pub fn set_tone(&mut self, value: f64) {
        self._argb = HctSolver::solve_to_argb(self.get_hue(), self.get_chroma(), value);

        let cam16 = Cam16::from(self._argb);

        self._hue = cam16.hue;
        self._chroma = cam16.chroma;
        self._tone = self._argb.as_lstar();
    }

    /// HCT of an ARGB colour.
    pub fn new(argb: Argb) -> Self {
        let cam16 = Cam16::from(argb);

        Self { _hue: cam16.hue, _chroma: cam16.chroma, _tone: argb.as_lstar(), _argb: argb }
    }

    /// Nearest in-gamut colour for a hue, chroma and tone.
    pub fn from(hue: f64, chroma: f64, tone: f64) -> Self {
        let argb = HctSolver::solve_to_argb(hue, chroma, tone);

        Self::new(argb)
    }

    /// The colour as it would appear under `vc` when viewed under the standard conditions.
    #[must_use]
    pub fn in_viewing_conditions(self, vc: &ViewingConditions) -> Self {
        let cam16 = Cam16::from(Argb::from(self));
        let viewed_in_vc = cam16.xyz_in_viewing_conditions(vc);

        let recast_in_vc = Cam16::from_xyz_in_viewing_conditions(
            viewed_in_vc.x,
            viewed_in_vc.y,
            viewed_in_vc.z,
            &ViewingConditions::standard(),
        );

        Self::from(recast_in_vc.hue, recast_in_vc.chroma, lstar_from_y(viewed_in_vc.y))
    }
}

impl fmt::Display for Hct {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "H{} C{} T{}", self.get_hue().round(), self.get_chroma().round(), self.get_tone().round())
    }
}

impl PartialEq for Hct {
    fn eq(&self, other: &Self) -> bool {
        self._argb == other._argb
    }
}

impl Eq for Hct {}

impl Hash for Hct {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self._hue.to_bits().hash(state);
        self._chroma.to_bits().hash(state);
        self._tone.to_bits().hash(state);
        self._argb.hash(state);
    }
}

impl From<Argb> for Hct {
    fn from(value: Argb) -> Self {
        Self::new(value)
    }
}

impl From<Hct> for Argb {
    fn from(value: Hct) -> Self {
        value._argb
    }
}
