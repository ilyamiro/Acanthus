pub use glam::{Affine2, Vec2};

/// An RGBA colour with `f32` channels in `0.0..=1.0`, not premultiplied.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Color {
    /// Red channel.
    pub r: f32,
    /// Green channel.
    pub g: f32,
    /// Blue channel.
    pub b: f32,
    /// Alpha channel (opacity).
    pub a: f32,
}

impl Color {
    /// Fully transparent black.
    pub const TRANSPARENT: Color = Color::rgba(0.0, 0.0, 0.0, 0.0);
    /// Opaque black.
    pub const BLACK: Color = Color::rgba(0.0, 0.0, 0.0, 1.0);
    /// Opaque white.
    pub const WHITE: Color = Color::rgba(1.0, 1.0, 1.0, 1.0);

    /// Builds a colour from its four channels.
    pub const fn rgba(r: f32, g: f32, b: f32, a: f32) -> Self {
        Color { r, g, b, a }
    }

    /// Builds an opaque colour from 8-bit channels.
    pub const fn rgb8(r: u8, g: u8, b: u8) -> Self {
        Color::rgba(r as f32 / 255.0, g as f32 / 255.0, b as f32 / 255.0, 1.0)
    }

    /// Builds an opaque colour from `0xRRGGBB`.
    pub const fn hex(rgb: u32) -> Self {
        Color::rgb8((rgb >> 16) as u8, (rgb >> 8) as u8, rgb as u8)
    }

    /// Builds a colour from `0xRRGGBBAA`.
    pub const fn hexa(rgba: u32) -> Self {
        let c = Color::hex(rgba >> 8);
        Color::rgba(c.r, c.g, c.b, (rgba & 0xff) as f32 / 255.0)
    }

    /// Parses `#rrggbb` or `#aarrggbb`; `None` for any other text.
    pub fn parse(s: &str) -> Option<Self> {
        let s = s.strip_prefix('#')?;
        let v = u32::from_str_radix(s, 16).ok()?;
        match s.len() {
            6 => Some(Color::hex(v)),
            8 => {
                let a = (v >> 24) as f32 / 255.0;
                Some(Color::hex(v & 0x00ff_ffff).with_alpha(a))
            }
            _ => None,
        }
    }

    /// The same colour with alpha replaced by `a`.
    pub const fn with_alpha(self, a: f32) -> Self {
        Color { a, ..self }
    }

    /// The same colour with alpha multiplied by `opacity`.
    pub fn fade(self, opacity: f32) -> Self {
        Color { a: self.a * opacity, ..self }
    }

    /// Linear interpolation of all four channels towards `to` by `t`.
    pub fn lerp(self, to: Color, t: f32) -> Self {
        Color {
            r: self.r + (to.r - self.r) * t,
            g: self.g + (to.g - self.g) * t,
            b: self.b + (to.b - self.b) * t,
            a: self.a + (to.a - self.a) * t,
        }
    }

    /// Alias of [`Color::lerp`].
    pub fn mix(self, to: Color, t: f32) -> Self {
        self.lerp(to, t)
    }

    /// Brightens by scaling the strongest channel by `factor` (clamped at 1.0); black becomes grey from `factor - 1`.
    pub fn lighten(self, factor: f32) -> Self {
        let max = self.r.max(self.g).max(self.b);
        if max <= 0.0 {
            let v = (factor - 1.0).clamp(0.0, 1.0);
            return Color { r: v, g: v, b: v, a: self.a };
        }
        let scale = (max * factor).min(1.0) / max;
        Color { r: self.r * scale, g: self.g * scale, b: self.b * scale, a: self.a }
    }

    /// Darkens by dividing the channels by `factor`; a `factor` of zero or less returns the colour unchanged.
    pub fn darken(self, factor: f32) -> Self {
        if factor <= 0.0 {
            return self;
        }
        Color {
            r: (self.r / factor).clamp(0.0, 1.0),
            g: (self.g / factor).clamp(0.0, 1.0),
            b: (self.b / factor).clamp(0.0, 1.0),
            a: self.a,
        }
    }

    /// The channels with alpha multiplied in, as `[r * a, g * a, b * a, a]`.
    pub fn premultiplied(self) -> [f32; 4] {
        [self.r * self.a, self.g * self.a, self.b * self.a, self.a]
    }
}

/// Function form of [`Color::lighten`].
pub fn lighten(color: Color, factor: f32) -> Color {
    color.lighten(factor)
}

/// Converts a logical length to device pixels for a UI scale: `value * scale`
/// rounded to the nearest whole pixel, half away from zero (the QML `Scaler.s` rule).
pub fn scaled(value: f32, scale: f32) -> f32 {
    (value * scale).round()
}

/// Function form of [`Color::darken`].
pub fn darken(color: Color, factor: f32) -> Color {
    color.darken(factor)
}

/// An axis-aligned rectangle: top-left corner plus size.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Rect {
    /// Left edge.
    pub x: f32,
    /// Top edge.
    pub y: f32,
    /// Width.
    pub w: f32,
    /// Height.
    pub h: f32,
}

impl Rect {
    /// Builds a rectangle from position and size.
    pub const fn new(x: f32, y: f32, w: f32, h: f32) -> Self {
        Rect { x, y, w, h }
    }

    /// The x coordinate of the right edge.
    pub fn right(&self) -> f32 {
        self.x + self.w
    }

    /// The y coordinate of the bottom edge.
    pub fn bottom(&self) -> f32 {
        self.y + self.h
    }

    /// Whether the width or height is zero or negative.
    pub fn is_empty(&self) -> bool {
        self.w <= 0.0 || self.h <= 0.0
    }

    /// Whether the point lies inside; the left and top edges count, the right and bottom edges do not.
    pub fn contains(&self, p: Vec2) -> bool {
        p.x >= self.x && p.y >= self.y && p.x < self.right() && p.y < self.bottom()
    }

    /// The overlap of two rectangles, empty (zero size) when they do not touch.
    pub fn intersect(&self, o: &Rect) -> Rect {
        let x = self.x.max(o.x);
        let y = self.y.max(o.y);
        let r = self.right().min(o.right());
        let b = self.bottom().min(o.bottom());
        Rect::new(x, y, (r - x).max(0.0), (b - y).max(0.0))
    }

    /// The rectangle moved by `d`.
    pub fn translate(&self, d: Vec2) -> Rect {
        Rect::new(self.x + d.x, self.y + d.y, self.w, self.h)
    }

    /// The rectangle shrunk by `m` on every side (grown when negative), never below zero size.
    pub fn inset(&self, m: f32) -> Rect {
        Rect::new(self.x + m, self.y + m, (self.w - 2.0 * m).max(0.0), (self.h - 2.0 * m).max(0.0))
    }

    /// Position and size multiplied by `s`.
    pub fn scale(&self, s: f32) -> Rect {
        Rect::new(self.x * s, self.y * s, self.w * s, self.h * s)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scaled_rounds_half_away_from_zero() {
        assert_eq!(scaled(10.0, 1.25), 13.0);
        assert_eq!(scaled(10.0, 1.5), 15.0);
        assert_eq!(scaled(-2.5, 1.0), -3.0);
        assert_eq!(scaled(7.0, 1.0), 7.0);
    }

    #[test]
    fn parses_hex_colors() {
        assert_eq!(Color::parse("#ff0000"), Some(Color::rgba(1.0, 0.0, 0.0, 1.0)));
        let c = Color::parse("#80ffffff").unwrap();
        assert!((c.a - 128.0 / 255.0).abs() < 1e-6);
        assert_eq!(Color::parse("fff"), None);
    }

    #[test]
    fn rect_intersection() {
        let a = Rect::new(0.0, 0.0, 10.0, 10.0);
        let b = Rect::new(5.0, 5.0, 10.0, 10.0);
        assert_eq!(a.intersect(&b), Rect::new(5.0, 5.0, 5.0, 5.0));
        assert!(a.intersect(&Rect::new(20.0, 20.0, 1.0, 1.0)).is_empty());
    }
}
