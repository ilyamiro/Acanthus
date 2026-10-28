use super::ViewingConditions;
use crate::{
    color::{Argb, Xyz},
    utils::math::signum,
};
use core::f64::consts::PI;

/// CAM16 colour appearance of a colour under given viewing conditions.
pub struct Cam16 {
    /// Hue in degrees.
    pub hue: f64,

    /// Chroma.
    pub chroma: f64,

    /// Lightness.
    pub j: f64,

    /// Brightness.
    pub q: f64,

    /// Colourfulness.
    pub m: f64,

    /// Saturation.
    pub s: f64,

    /// Lightness in the CAM16-UCS space.
    pub jstar: f64,

    /// Red-green coordinate in CAM16-UCS.
    pub astar: f64,

    /// Yellow-blue coordinate in CAM16-UCS.
    pub bstar: f64,
}

impl Cam16 {
    /// Euclidean distance to `other` in CAM16-UCS.
    pub fn distance(&self, other: &Self) -> f64 {
        let d_j = self.jstar - other.jstar;
        let d_a = self.astar - other.astar;
        let d_b = self.bstar - other.bstar;
        let d_eprime = d_b.mul_add(d_b, d_j.mul_add(d_j, d_a * d_a)).sqrt();

        1.41 * d_eprime.powf(0.63)
    }

    /// Appearance of `argb` under `viewing_conditions`.
    pub fn fromi32_in_viewing_conditions(argb: Argb, viewing_conditions: &ViewingConditions) -> Self {
        let Xyz { x, y, z } = Xyz::from(argb);

        Self::from_xyz_in_viewing_conditions(x, y, z, viewing_conditions)
    }

    /// Appearance of an XYZ colour under `viewing_conditions`.
    pub fn from_xyz_in_viewing_conditions(x: f64, y: f64, z: f64, viewing_conditions: &ViewingConditions) -> Self {
        let (r_c, g_c, b_c) = (
            0.051461f64.mul_add(-z, 0.401288f64.mul_add(x, 0.650173 * y)),
            0.045854f64.mul_add(z, (-0.250268f64).mul_add(x, 1.204414 * y)),
            0.953127f64.mul_add(z, (-0.002079f64).mul_add(x, 0.048952 * y)),
        );

        let r_d = viewing_conditions.rgb_d[0] * r_c;
        let g_d = viewing_conditions.rgb_d[1] * g_c;
        let b_d = viewing_conditions.rgb_d[2] * b_c;

        let (r_af, g_af, b_af) = (
            (viewing_conditions.fl * r_d.abs() / 100.0).powf(0.42),
            (viewing_conditions.fl * g_d.abs() / 100.0).powf(0.42),
            (viewing_conditions.fl * b_d.abs() / 100.0).powf(0.42),
        );
        let r_a = signum(r_d) * 400.0 * r_af / (r_af + 27.13);
        let g_a = signum(g_d) * 400.0 * g_af / (g_af + 27.13);
        let b_a = signum(b_d) * 400.0 * b_af / (b_af + 27.13);

        let (a, b, u, p2) = (
            (11.0f64.mul_add(r_a, -12.0 * g_a) + b_a) / 11.0,
            2.0f64.mul_add(-b_a, r_a + g_a) / 9.0,
            21.0f64.mul_add(b_a, 20.0f64.mul_add(r_a, 20.0 * g_a)) / 20.0,
            (40.0f64.mul_add(r_a, 20.0 * g_a) + b_a) / 20.0,
        );

        let atan2 = b.atan2(a);
        let atan_degrees = atan2.to_degrees();
        let hue = if atan_degrees < 0.0 {
            atan_degrees + 360.0
        } else if atan_degrees >= 360.0 {
            atan_degrees - 360.0
        } else {
            atan_degrees
        };
        let hue_radians = hue.to_radians();

        assert!((0.0..360.0).contains(&hue), "hue was really {hue}");

        let ac = p2 * viewing_conditions.nbb;

        let j = 100.0 * (ac / viewing_conditions.aw).powf(viewing_conditions.c * viewing_conditions.z);

        let q = (4.0 / viewing_conditions.c)
            * (j / 100.0).sqrt()
            * (viewing_conditions.aw + 4.0)
            * (viewing_conditions.f_lroot);

        let hue_prime = if hue < 20.14 { hue + 360.0 } else { hue };
        let e_hue = (1.0 / 4.0) * ((hue_prime.to_radians() + 2.0).cos() + 3.8);
        let p1 = 50000.0 / 13.0 * e_hue * viewing_conditions.n_c * viewing_conditions.ncb;
        let t = p1 * a.hypot(b) / (u + 0.305);

        let alpha = t.powf(0.9) * (1.64 - 0.29f64.powf(viewing_conditions.background_ytowhite_point_y)).powf(0.73);

        let c = alpha * (j / 100.0).sqrt();
        let m = c * viewing_conditions.f_lroot;
        let s = 50.0 * ((alpha * viewing_conditions.c) / (viewing_conditions.aw + 4.0)).sqrt();

        let (jstar, mstar) =
            (100.0f64.mul_add(0.007, 1.0) * j / 0.007f64.mul_add(j, 1.0), (0.0228 * m).ln_1p() / 0.0228);

        let (astar, bstar) = (mstar * hue_radians.cos(), mstar * hue_radians.sin());

        Self { hue, chroma: c, j, q, m, s, jstar, astar, bstar }
    }

    /// Colour from lightness, chroma and hue under the standard conditions.
    pub fn from_jch(j: f64, c: f64, h: f64) -> Self {
        Self::from_jch_in_viewing_conditions(j, c, h, &ViewingConditions::s_rgb())
    }

    /// Colour from lightness, chroma and hue under `viewing_conditions`.
    pub fn from_jch_in_viewing_conditions(j: f64, c: f64, h: f64, viewing_conditions: &ViewingConditions) -> Self {
        let q = (4.0 / viewing_conditions.c)
            * (j / 100.0).sqrt()
            * (viewing_conditions.aw + 4.0)
            * (viewing_conditions.f_lroot);
        let m = c * viewing_conditions.f_lroot;
        let alpha = c / (j / 100.0).sqrt();
        let s = 50.0 * ((alpha * viewing_conditions.c) / (viewing_conditions.aw + 4.0)).sqrt();

        let hue_radians = h.to_radians();
        let (jstar, mstar) = (
            100.0_f64.mul_add(0.007, 1.0) * j / 0.007_f64.mul_add(j, 1.0),
            1.0 / 0.0228 * 0.0228_f64.mul_add(m, 1.0).ln(),
        );

        let (astar, bstar) = (mstar * hue_radians.cos(), mstar * hue_radians.sin());

        Self { hue: h, chroma: c, j, q, m, s, jstar, astar, bstar }
    }

    /// Colour from CAM16-UCS coordinates under the standard conditions.
    pub fn from_ucs(jstar: f64, astar: f64, bstar: f64) -> Self {
        Self::from_ucs_in_viewing_conditions(jstar, astar, bstar, &ViewingConditions::standard())
    }

    /// Colour from CAM16-UCS coordinates under `viewing_conditions`.
    pub fn from_ucs_in_viewing_conditions(
        jstar: f64,
        astar: f64,
        bstar: f64,
        viewing_conditions: &ViewingConditions,
    ) -> Self {
        let a = astar;
        let b = bstar;
        let m = a.hypot(b);
        let m = (m * 0.0228).exp_m1() / 0.0228;
        let c = m / viewing_conditions.f_lroot;
        let h = b.atan2(a) * (180.0 / PI);
        let h = if h < 0.0 { h + 360.0 } else { h };
        let j = jstar / (jstar - 100.0).mul_add(-0.007, 1.0);

        Self::from_jch_in_viewing_conditions(j, c, h, viewing_conditions)
    }

    /// ARGB of this appearance under `viewing_conditions`.
    pub fn viewed(&self, viewing_conditions: &ViewingConditions) -> Argb {
        let xyz = self.xyz_in_viewing_conditions(viewing_conditions);

        xyz.into()
    }

    /// XYZ of this appearance under `viewing_conditions`.
    pub fn xyz_in_viewing_conditions(&self, viewing_conditions: &ViewingConditions) -> Xyz {
        let alpha = if self.chroma == 0.0 || self.j == 0.0 { 0.0 } else { self.chroma / (self.j / 100.0).sqrt() };

        let t =
            (alpha / (1.64 - 0.29_f64.powf(viewing_conditions.background_ytowhite_point_y)).powf(0.73)).powf(1.0 / 0.9);
        let h_rad = self.hue.to_radians();

        let e_hue = 0.25 * ((h_rad + 2.0).cos() + 3.8);
        let ac = viewing_conditions.aw * (self.j / 100.0).powf(1.0 / viewing_conditions.c / viewing_conditions.z);
        let p1 = e_hue * (50000.0 / 13.0) * viewing_conditions.n_c * viewing_conditions.ncb;

        let p2 = ac / viewing_conditions.nbb;

        let (h_sin, h_cos) = (h_rad.sin(), h_rad.cos());

        let gamma = 23.0 * (p2 + 0.305) * t / (108.0 * t).mul_add(h_sin, 23.0f64.mul_add(p1, 11.0 * t * h_cos));
        let a = gamma * h_cos;
        let b = gamma * h_sin;
        let (r_a, g_a, b_a) = (
            288.0f64.mul_add(b, 460.0f64.mul_add(p2, 451.0 * a)) / 1403.0,
            261.0f64.mul_add(-b, 460.0f64.mul_add(p2, -891.0 * a)) / 1403.0,
            6300.0f64.mul_add(-b, 460.0f64.mul_add(p2, -220.0 * a)) / 1403.0,
        );

        let (r_cbase, g_cbase, b_cbase) = (
            0.0f64.max((27.13 * r_a.abs()) / (400.0 - r_a.abs())),
            0.0f64.max((27.13 * g_a.abs()) / (400.0 - g_a.abs())),
            0.0f64.max((27.13 * b_a.abs()) / (400.0 - b_a.abs())),
        );

        let (r_c, g_c, b_c) = (
            signum(r_a) * (100.0 / viewing_conditions.fl) * r_cbase.powf(1.0 / 0.42),
            signum(g_a) * (100.0 / viewing_conditions.fl) * g_cbase.powf(1.0 / 0.42),
            signum(b_a) * (100.0 / viewing_conditions.fl) * b_cbase.powf(1.0 / 0.42),
        );

        let r_f = r_c / viewing_conditions.rgb_d[0];
        let g_f = g_c / viewing_conditions.rgb_d[1];
        let b_f = b_c / viewing_conditions.rgb_d[2];

        let (x, y, z) = (
            0.14918677f64.mul_add(b_f, 1.86206786f64.mul_add(r_f, -1.01125463 * g_f)),
            0.00897398f64.mul_add(-b_f, 0.38752654f64.mul_add(r_f, 0.62144744 * g_f)),
            1.04996444f64.mul_add(b_f, (-0.01584150f64).mul_add(r_f, -0.03412294 * g_f)),
        );

        Xyz::new(x, y, z)
    }
}

impl From<Argb> for Cam16 {
    fn from(argb: Argb) -> Self {
        Self::fromi32_in_viewing_conditions(argb, &ViewingConditions::s_rgb())
    }
}

impl From<Cam16> for Argb {
    fn from(val: Cam16) -> Self {
        val.viewed(&ViewingConditions::s_rgb())
    }
}
