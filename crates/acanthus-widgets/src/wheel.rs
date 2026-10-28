//! Mouse wheel stepping: turns wheel deltas into whole notches the way Qt counts angle deltas
//! (120 units per notch).

/// Units per wheel notch (Qt's angle delta of one notch).
pub const WHEEL_STEP: f32 = 120.0;
/// Units per pixel of a Wayland axis value, so a 10-pixel notch gives one [`WHEEL_STEP`].
pub const WHEEL_UNITS_PER_ABSOLUTE: f32 = 12.0;
/// Idle time after which widgets reset their [`Wheel`], in milliseconds.
pub const WHEEL_RESET_MS: u64 = 200;

/// Converts a vertical scroll delta in pixels to wheel units, positive when scrolling up.
pub fn wheel_units(delta_y: f32) -> f32 {
    -delta_y * WHEEL_UNITS_PER_ABSOLUTE
}

/// Accumulator that turns small wheel or touchpad deltas into whole notches.
#[derive(Debug, Default)]
pub struct Wheel {
    accumulator: f32,
}

impl Wheel {
    /// Adds `delta` units and returns the whole notches completed (signed), keeping the remainder.
    pub fn feed(&mut self, delta: f32) -> i32 {
        self.accumulator += delta;
        if self.accumulator.abs() < WHEEL_STEP {
            return 0;
        }
        let steps = (self.accumulator / WHEEL_STEP).trunc();
        self.accumulator %= WHEEL_STEP;
        steps as i32
    }

    /// Drops any partial notch.
    pub fn reset(&mut self) {
        self.accumulator = 0.0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wheel_accumulates_to_whole_steps() {
        let mut wheel = Wheel::default();
        assert_eq!(wheel.feed(60.0), 0);
        assert_eq!(wheel.feed(60.0), 1);
        assert_eq!(wheel.feed(-240.0), -2);
        assert_eq!(wheel.feed(-60.0), 0);
        wheel.feed(60.0);
        wheel.reset();
        assert_eq!(wheel.feed(60.0), 0);
    }

    #[test]
    fn units_flip_the_sign() {
        assert_eq!(wheel_units(10.0), -120.0);
        assert_eq!(wheel_units(-10.0), 120.0);
    }
}
