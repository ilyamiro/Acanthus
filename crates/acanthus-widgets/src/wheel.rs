pub const WHEEL_STEP: f32 = 120.0;
pub const WHEEL_UNITS_PER_ABSOLUTE: f32 = 12.0;
pub const WHEEL_RESET_MS: u64 = 200;

pub fn wheel_units(delta_y: f32) -> f32 {
    -delta_y * WHEEL_UNITS_PER_ABSOLUTE
}

#[derive(Debug, Default)]
pub struct Wheel {
    accumulator: f32,
}

impl Wheel {
    pub fn feed(&mut self, delta: f32) -> i32 {
        self.accumulator += delta;
        if self.accumulator.abs() < WHEEL_STEP {
            return 0;
        }
        let steps = (self.accumulator / WHEEL_STEP).trunc();
        self.accumulator %= WHEEL_STEP;
        steps as i32
    }

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
