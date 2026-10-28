use crate::color::{Argb, Lab};

/// Converts colours to points of a colour space and measures distance between them.
pub trait PointProvider {
    /// Point of `argb` in the provider space.
    fn lab_from_int(argb: &Argb) -> Lab;
    /// Colour of a point in the provider space.
    fn lab_to_int(lab: &Lab) -> Argb;
    /// Squared distance between two points.
    fn distance(one: &Lab, two: &Lab) -> f64;
}
