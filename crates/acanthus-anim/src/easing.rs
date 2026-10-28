use std::f32::consts::PI;

pub const DEFAULT_OVERSHOOT: f32 = 1.70158;

#[derive(Debug, Clone, Copy, PartialEq, Default)]
/// Qt-exact easing curve, the equivalent of `Easing.Type` in QML; the `*Back` variants carry the overshoot.
pub enum Easing {
    #[default]
    /// Constant speed.
    Linear,
    /// Quadratic ease in.
    InQuad,
    /// Quadratic ease out.
    OutQuad,
    /// Quadratic ease in and out.
    InOutQuad,
    /// Cubic ease in.
    InCubic,
    /// Cubic ease out.
    OutCubic,
    /// Cubic ease in and out.
    InOutCubic,
    /// Quartic ease in.
    InQuart,
    /// Quartic ease out.
    OutQuart,
    /// Quartic ease in and out.
    InOutQuart,
    /// Quintic ease in.
    InQuint,
    /// Quintic ease out.
    OutQuint,
    /// Quintic ease in and out.
    InOutQuint,
    /// Sinusoidal ease in.
    InSine,
    /// Sinusoidal ease out.
    OutSine,
    /// Sinusoidal ease in and out.
    InOutSine,
    /// Exponential ease in.
    InExpo,
    /// Exponential ease out.
    OutExpo,
    /// Exponential ease in and out.
    InOutExpo,
    /// Circular ease in.
    InCirc,
    /// Circular ease out.
    OutCirc,
    /// Circular ease in and out.
    InOutCirc,
    /// Ease in that pulls back by the given overshoot before moving forward.
    InBack(f32),
    /// Ease out that overshoots the target by the given amount and settles back.
    OutBack(f32),
    /// Ease in and out with the given overshoot at both ends.
    InOutBack(f32),
}

impl Easing {
    /// `InBack` with the Qt default overshoot of 1.70158.
    pub const IN_BACK: Easing = Easing::InBack(DEFAULT_OVERSHOOT);
    /// `OutBack` with the Qt default overshoot of 1.70158.
    pub const OUT_BACK: Easing = Easing::OutBack(DEFAULT_OVERSHOOT);
    /// `InOutBack` with the Qt default overshoot of 1.70158.
    pub const IN_OUT_BACK: Easing = Easing::InOutBack(DEFAULT_OVERSHOOT);

    /// Maps linear progress `t` (clamped to 0..1) to eased progress.
    pub fn apply(self, t: f32) -> f32 {
        let t = t.clamp(0.0, 1.0);
        match self {
            Easing::Linear => t,
            Easing::InQuad => t * t,
            Easing::OutQuad => -t * (t - 2.0),
            Easing::InOutQuad => in_out(t, |t| t * t),
            Easing::InCubic => t * t * t,
            Easing::OutCubic => out(t, |t| t * t * t),
            Easing::InOutCubic => in_out(t, |t| t * t * t),
            Easing::InQuart => t.powi(4),
            Easing::OutQuart => out(t, |t| t.powi(4)),
            Easing::InOutQuart => in_out(t, |t| t.powi(4)),
            Easing::InQuint => t.powi(5),
            Easing::OutQuint => out(t, |t| t.powi(5)),
            Easing::InOutQuint => in_out(t, |t| t.powi(5)),
            Easing::InSine => {
                if t == 1.0 {
                    1.0
                } else {
                    1.0 - (t * PI / 2.0).cos()
                }
            }
            Easing::OutSine => (t * PI / 2.0).sin(),
            Easing::InOutSine => -0.5 * ((PI * t).cos() - 1.0),
            Easing::InExpo => in_expo(t),
            Easing::OutExpo => out_expo(t),
            Easing::InOutExpo => {
                if t == 0.0 || t == 1.0 {
                    return t;
                }
                let t2 = t * 2.0;
                if t2 < 1.0 {
                    0.5 * 2f32.powf(10.0 * (t2 - 1.0)) - 0.0005
                } else {
                    0.5 * 1.0005 * (-(2f32.powf(-10.0 * (t2 - 1.0))) + 2.0)
                }
            }
            Easing::InCirc => -((1.0 - t * t).sqrt() - 1.0),
            Easing::OutCirc => {
                let t = t - 1.0;
                (1.0 - t * t).sqrt()
            }
            Easing::InOutCirc => {
                let t2 = t * 2.0;
                if t2 < 1.0 {
                    -0.5 * ((1.0 - t2 * t2).sqrt() - 1.0)
                } else {
                    let t2 = t2 - 2.0;
                    0.5 * ((1.0 - t2 * t2).sqrt() + 1.0)
                }
            }
            Easing::InBack(s) => t * t * ((s + 1.0) * t - s),
            Easing::OutBack(s) => {
                let t = t - 1.0;
                t * t * ((s + 1.0) * t + s) + 1.0
            }
            Easing::InOutBack(s) => {
                let s = s * 1.525;
                let t2 = t * 2.0;
                if t2 < 1.0 {
                    0.5 * (t2 * t2 * ((s + 1.0) * t2 - s))
                } else {
                    let t2 = t2 - 2.0;
                    0.5 * (t2 * t2 * ((s + 1.0) * t2 + s) + 2.0)
                }
            }
        }
    }
}

fn out(t: f32, f: impl Fn(f32) -> f32) -> f32 {
    1.0 - f(1.0 - t)
}

fn in_out(t: f32, f: impl Fn(f32) -> f32) -> f32 {
    if t < 0.5 { f(t * 2.0) / 2.0 } else { 1.0 - f((1.0 - t) * 2.0) / 2.0 }
}

fn in_expo(t: f32) -> f32 {
    if t == 0.0 || t == 1.0 { t } else { 2f32.powf(10.0 * (t - 1.0)) - 0.001 }
}

fn out_expo(t: f32) -> f32 {
    if t == 1.0 { 1.0 } else { 1.001 * (-(2f32.powf(-10.0 * t)) + 1.0) }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ALL: [Easing; 25] = [
        Easing::Linear,
        Easing::InQuad,
        Easing::OutQuad,
        Easing::InOutQuad,
        Easing::InCubic,
        Easing::OutCubic,
        Easing::InOutCubic,
        Easing::InQuart,
        Easing::OutQuart,
        Easing::InOutQuart,
        Easing::InQuint,
        Easing::OutQuint,
        Easing::InOutQuint,
        Easing::InSine,
        Easing::OutSine,
        Easing::InOutSine,
        Easing::InExpo,
        Easing::OutExpo,
        Easing::InOutExpo,
        Easing::InCirc,
        Easing::OutCirc,
        Easing::InOutCirc,
        Easing::IN_BACK,
        Easing::OUT_BACK,
        Easing::IN_OUT_BACK,
    ];

    #[test]
    fn endpoints_are_fixed() {
        for e in ALL {
            assert!(e.apply(0.0).abs() < 1e-3, "{e:?} at 0 = {}", e.apply(0.0));
            assert!((e.apply(1.0) - 1.0).abs() < 1e-3, "{e:?} at 1 = {}", e.apply(1.0));
        }
    }

    #[test]
    fn out_cubic_matches_qt() {
        assert!((Easing::OutCubic.apply(0.5) - 0.875).abs() < 1e-6);
    }

    #[test]
    fn out_back_overshoots() {
        let peak = (1..100).map(|i| Easing::OUT_BACK.apply(i as f32 / 100.0)).fold(0.0, f32::max);
        assert!(peak > 1.05);
    }
}
