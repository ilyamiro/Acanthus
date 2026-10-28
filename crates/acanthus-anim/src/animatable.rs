use acanthus_core::{Color, Vec2};

/// A value that can be animated by interpolating up to four `f32` components.
pub trait Animatable: Clone + PartialEq + 'static {
    /// Splits the value into its components, unused ones are zero.
    fn components(&self) -> [f32; 4];
    /// Rebuilds the value from components produced by [`Animatable::components`].
    fn from_components(c: [f32; 4]) -> Self;

    /// Linear interpolation from `self` to `to` at progress `t`.
    fn lerp(&self, to: &Self, t: f32) -> Self {
        let a = self.components();
        let b = to.components();
        Self::from_components(std::array::from_fn(|i| a[i] + (b[i] - a[i]) * t))
    }
}

impl Animatable for f32 {
    fn components(&self) -> [f32; 4] {
        [*self, 0.0, 0.0, 0.0]
    }

    fn from_components(c: [f32; 4]) -> Self {
        c[0]
    }
}

impl Animatable for Vec2 {
    fn components(&self) -> [f32; 4] {
        [self.x, self.y, 0.0, 0.0]
    }

    fn from_components(c: [f32; 4]) -> Self {
        Vec2::new(c[0], c[1])
    }
}

impl Animatable for Color {
    fn components(&self) -> [f32; 4] {
        [self.r, self.g, self.b, self.a]
    }

    fn from_components(c: [f32; 4]) -> Self {
        Color::rgba(c[0], c[1], c[2], c[3])
    }
}
