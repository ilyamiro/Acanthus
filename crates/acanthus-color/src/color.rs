use crate::{Error, utils::math::matrix_multiply};
use core::{fmt, str::FromStr};

/// Matrix converting linear sRGB to XYZ.
pub const SRGB_TO_XYZ: [[f64; 3]; 3] =
    [[0.41233895, 0.35762064, 0.18051042], [0.2126, 0.7152, 0.0722], [0.01932141, 0.11916382, 0.95034478]];
/// Matrix converting XYZ to linear sRGB.
pub const XYZ_TO_SRGB: [[f64; 3]; 3] = [
    [3.2413774792388685, -1.5376652402851851, -0.49885366846268053],
    [-0.9691452513005321, 1.8758853451067872, 0.04156585616912061],
    [0.05562093689691305, -0.20395524564742123, 1.0571799111220335],
];
/// XYZ of the D65 white point.
pub const WHITE_POINT_D65: [f64; 3] = [95.047, 100.0, 108.883];

/// An opaque sRGB colour.
#[derive(Debug, Default, Clone, Copy)]
pub struct Rgb {
    /// Red.
    pub red: u8,
    /// Green.
    pub green: u8,
    /// Blue.
    pub blue: u8,
}

/// An sRGB colour with alpha.
#[derive(Debug, Default, Clone, Copy, Hash, PartialEq, Eq, PartialOrd, Ord)]
pub struct Argb {
    /// Alpha.
    pub alpha: u8,
    /// Red.
    pub red: u8,
    /// Green.
    pub green: u8,
    /// Blue.
    pub blue: u8,
}

/// A linear-light RGB colour on a 0 to 100 scale.
#[derive(Debug, Default, Clone, Copy)]
pub struct LinearRgb {
    /// Red.
    pub red: f64,
    /// Green.
    pub green: f64,
    /// Blue.
    pub blue: f64,
}

/// A CIE XYZ colour.
#[derive(Debug, Default, Clone, Copy)]
pub struct Xyz {
    /// X.
    pub x: f64,
    /// Y.
    pub y: f64,
    /// Z.
    pub z: f64,
}

/// A CIE Lab colour.
#[derive(Debug, Default, Clone, Copy)]
pub struct Lab {
    /// Lightness.
    pub l: f64,
    /// Green-red axis.
    pub a: f64,
    /// Blue-yellow axis.
    pub b: f64,
}

impl From<Rgb> for Argb {
    fn from(Rgb { red, green, blue }: Rgb) -> Self {
        Self { alpha: 255, red, green, blue }
    }
}

impl From<LinearRgb> for Argb {
    fn from(linear: LinearRgb) -> Self {
        let r = delinearized(linear.red);
        let g = delinearized(linear.green);
        let b = delinearized(linear.blue);

        Rgb::new(r, g, b).into()
    }
}

impl From<Xyz> for Argb {
    fn from(Xyz { x, y, z }: Xyz) -> Self {
        let matrix = XYZ_TO_SRGB;

        let (linear_r, linear_g, linear_b) = (
            matrix[0][2].mul_add(z, matrix[0][0].mul_add(x, matrix[0][1] * y)),
            matrix[1][2].mul_add(z, matrix[1][0].mul_add(x, matrix[1][1] * y)),
            matrix[2][2].mul_add(z, matrix[2][0].mul_add(x, matrix[2][1] * y)),
        );

        let r = delinearized(linear_r);
        let g = delinearized(linear_g);
        let b = delinearized(linear_b);

        Rgb::new(r, g, b).into()
    }
}

impl From<Argb> for Xyz {
    fn from(Argb { alpha: _, red, green, blue }: Argb) -> Self {
        let r = linearized(red);
        let g = linearized(green);
        let b = linearized(blue);

        let [x, y, z] = matrix_multiply([r, g, b], SRGB_TO_XYZ);

        Self { x, y, z }
    }
}

impl From<Lab> for Argb {
    fn from(Lab { l, a, b }: Lab) -> Self {
        let white_point = WHITE_POINT_D65;

        let fy = (l + 16.0) / 116.0;
        let fx = a / 500.0 + fy;
        let fz = fy - b / 200.0;

        let x_normalized = lab_invf(fx);
        let y_normalized = lab_invf(fy);
        let z_normalized = lab_invf(fz);

        let x = x_normalized * white_point[0];
        let y = y_normalized * white_point[1];
        let z = z_normalized * white_point[2];

        Xyz::new(x, y, z).into()
    }
}

impl From<Argb> for Lab {
    fn from(Argb { alpha: _, red, green, blue }: Argb) -> Self {
        let linear_r = linearized(red);
        let linear_g = linearized(green);
        let linear_b = linearized(blue);

        let matrix = SRGB_TO_XYZ;

        let (x, y, z) = (
            matrix[0][2].mul_add(linear_b, matrix[0][0].mul_add(linear_r, matrix[0][1] * linear_g)),
            matrix[1][2].mul_add(linear_b, matrix[1][0].mul_add(linear_r, matrix[1][1] * linear_g)),
            matrix[2][2].mul_add(linear_b, matrix[2][0].mul_add(linear_r, matrix[2][1] * linear_g)),
        );

        let white_point = WHITE_POINT_D65;

        let x_normalized = x / white_point[0];
        let y_normalized = y / white_point[1];
        let z_normalized = z / white_point[2];

        let fx = lab_f(x_normalized);
        let fy = lab_f(y_normalized);
        let fz = lab_f(z_normalized);

        let l = 116.0f64.mul_add(fy, -16.0);
        let a = 500.0 * (fx - fy);
        let b = 200.0 * (fy - fz);

        Self { l, a, b }
    }
}

const HASH: char = '#';

impl FromStr for Argb {
    type Err = Error;

    fn from_str(hex: &str) -> Result<Self, Self::Err> {
        let hex = hex.strip_prefix(HASH).unwrap_or(hex);

        if ![3, 6, 8].contains(&hex.len()) {
            return Err(Error::ParseRGB);
        }

        let hex_str = if hex.len() == 3 {
            format!(
                "FF{a}{a}{b}{b}{c}{c}",
                a = hex.get(..1).unwrap(),
                b = hex.get(1..2).unwrap(),
                c = hex.get(2..3).unwrap()
            )
        } else if hex.len() == 6 {
            format!("FF{hex}")
        } else {
            hex.to_string()
        };

        let hex_digit = u32::from_str_radix(&hex_str, 16).map_err(|_| Error::ParseRGB)?;

        Ok(Self::from_u32(hex_digit))
    }
}

impl Xyz {
    /// Creates an XYZ colour.
    pub const fn new(x: f64, y: f64, z: f64) -> Self {
        Self { x, y, z }
    }
}

impl Lab {
    /// Creates a Lab colour.
    pub const fn new(l: f64, a: f64, b: f64) -> Self {
        Self { l, a, b }
    }
}

impl Rgb {
    /// Creates an opaque colour.
    pub const fn new(red: u8, green: u8, blue: u8) -> Self {
        Self { red, green, blue }
    }
}

impl Argb {
    /// Creates a colour from its channels.
    pub const fn new(alpha: u8, red: u8, green: u8, blue: u8) -> Self {
        Self { alpha, red, green, blue }
    }

    /// Unpacks a `0xAARRGGBB` value.
    pub const fn from_u32(value: u32) -> Self {
        Self {
            alpha: ((value >> 24) & 0xFF) as u8,
            red: ((value >> 16) & 0xFF) as u8,
            green: ((value >> 8) & 0xFF) as u8,
            blue: ((value) & 0xFF) as u8,
        }
    }

    /// Gray with the given L*.
    pub fn from_lstar(lstar: f64) -> Self {
        let y = y_from_lstar(lstar);
        let component = delinearized(y);

        Rgb::new(component, component, component).into()
    }

    /// L* of the colour.
    pub fn as_lstar(&self) -> f64 {
        116.0f64.mul_add(lab_f(Xyz::from(*self).y / 100.0), -16.0)
    }

    fn hex(number: u8) -> String {
        let string = format!("{number:x}");

        if string.len() == 1 { String::from("0") + &string } else { string }
    }

    /// Hex digits `rrggbb` without a leading `#`.
    pub fn to_hex(&self) -> String {
        format!("{}{}{}", Self::hex(self.red), Self::hex(self.green), Self::hex(self.blue))
    }

    /// Hex digits with a leading `#`.
    pub fn to_hex_with_pound(&self) -> String {
        format!("#{}{}{}", Self::hex(self.red), Self::hex(self.green), Self::hex(self.blue))
    }
}

impl fmt::Display for Argb {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.to_hex_with_pound())
    }
}

/// Luminance Y (0 to 100) of a tone.
pub fn y_from_lstar(lstar: f64) -> f64 {
    100.0 * lab_invf((lstar + 16.0) / 116.0)
}

/// Tone (L*) of a luminance Y (0 to 100).
pub fn lstar_from_y(y: f64) -> f64 {
    lab_f(y / 100.0).mul_add(116.0, -16.0)
}

/// Linear light (0 to 100) of an 8-bit sRGB component.
pub fn linearized(rgb_component: u8) -> f64 {
    let normalized = f64::from(rgb_component) / 255.0;

    if normalized <= 0.040449936 {
        normalized / 12.92 * 100.0
    } else {
        ((normalized + 0.055) / 1.055).powf(2.4) * 100.0
    }
}

/// 8-bit sRGB component of a linear light value (0 to 100).
pub fn delinearized(rgb_component: f64) -> u8 {
    let normalized = rgb_component / 100.0;
    let delinearized =
        if normalized <= 0.0031308 { normalized * 12.92 } else { 1.055f64.mul_add(normalized.powf(1.0 / 2.4), -0.055) };

    ((delinearized * 255.0).round() as u8).clamp(0, 255)
}

fn lab_f(t: f64) -> f64 {
    let e = 216.0 / 24389.0;
    let kappa: f64 = 24389.0 / 27.0;

    if t > e { t.cbrt() } else { kappa.mul_add(t, 16.0) / 116.0 }
}

fn lab_invf(ft: f64) -> f64 {
    let e = 216.0 / 24389.0;
    let kappa = 24389.0 / 27.0;
    let ft3 = ft * ft * ft;

    if ft3 > e { ft3 } else { 116.0f64.mul_add(ft, -16.0) / kappa }
}
