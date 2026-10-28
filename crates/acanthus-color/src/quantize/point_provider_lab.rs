use super::PointProvider;
use crate::color::{Argb, Lab};

/// Point provider in CIE Lab space.
pub struct PointProviderLab;

impl PointProvider for PointProviderLab {
    fn lab_from_int(argb: &Argb) -> Lab {
        (*argb).into()
    }

    fn lab_to_int(lab: &Lab) -> Argb {
        (*lab).into()
    }

    fn distance(one: &Lab, two: &Lab) -> f64 {
        (one.b - two.b).mul_add(one.b - two.b, (one.l - two.l).mul_add(one.l - two.l, (one.a - two.a).powi(2)))
    }
}
