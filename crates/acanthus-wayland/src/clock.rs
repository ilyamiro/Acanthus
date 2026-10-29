use std::time::{Duration, Instant};

const MAX_STEP: f32 = 0.1;

#[derive(Debug, Default)]
pub(crate) struct FrameClock {
    last: Option<Instant>,
    last_callback: Option<u32>,
    callback: Option<u32>,
}

fn later(a: u32, b: u32) -> bool {
    a != b && a.wrapping_sub(b) < u32::MAX / 2
}

impl FrameClock {
    pub fn callback(&mut self, time_ms: u32) {
        if self.callback.is_none_or(|seen| later(time_ms, seen)) {
            self.callback = Some(time_ms);
        }
    }

    pub fn step(&mut self, active: bool, waiting: bool, now: Instant) -> Option<f32> {
        let callback = self.callback.take();
        if !active {
            self.last = None;
            self.last_callback = None;
            return None;
        }
        if callback.is_none() && waiting && self.last.is_some() {
            return None;
        }
        let elapsed = || self.last.map_or(Duration::ZERO, |t| now - t);
        let dt = match (callback, self.last_callback) {
            (Some(at), Some(prev)) if later(at, prev) => Duration::from_millis(u64::from(at.wrapping_sub(prev))),
            (Some(_), Some(_)) => Duration::ZERO,
            _ => elapsed(),
        };
        if callback.is_none_or(|at| self.last_callback.is_none_or(|prev| later(at, prev))) {
            self.last_callback = callback;
        }
        self.last = Some(now);
        Some(dt.as_secs_f32().min(MAX_STEP))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ms(n: u64) -> Duration {
        Duration::from_millis(n)
    }

    #[test]
    fn idle_clock_does_not_tick() {
        let mut clock = FrameClock::default();
        clock.callback(10);
        assert_eq!(clock.step(false, false, Instant::now()), None);
    }

    #[test]
    fn first_tick_is_zero() {
        let mut clock = FrameClock::default();
        assert_eq!(clock.step(true, true, Instant::now()), Some(0.0));
    }

    #[test]
    fn input_between_frames_does_not_tick() {
        let mut clock = FrameClock::default();
        let t0 = Instant::now();
        clock.step(true, true, t0);
        assert_eq!(clock.step(true, true, t0 + ms(3)), None);
        assert_eq!(clock.step(true, true, t0 + ms(7)), None);
    }

    #[test]
    fn steps_by_callback_timestamps_not_arrival() {
        let mut clock = FrameClock::default();
        let t0 = Instant::now();
        clock.callback(1000);
        clock.step(true, true, t0);
        clock.callback(1016);
        let dt = clock.step(true, true, t0 + ms(25)).unwrap();
        assert!((dt - 0.016).abs() < 1e-6);
    }

    #[test]
    fn several_surfaces_take_the_latest_timestamp() {
        let mut clock = FrameClock::default();
        let t0 = Instant::now();
        clock.callback(1000);
        clock.step(true, true, t0);
        clock.callback(1017);
        clock.callback(1015);
        let dt = clock.step(true, true, t0 + ms(17)).unwrap();
        assert!((dt - 0.017).abs() < 1e-6);
        clock.callback(1016);
        assert_eq!(clock.step(true, true, t0 + ms(18)), Some(0.0));
        clock.callback(1033);
        let dt = clock.step(true, true, t0 + ms(34)).unwrap();
        assert!((dt - 0.016).abs() < 1e-6);
    }

    #[test]
    fn timestamps_wrap_around() {
        let mut clock = FrameClock::default();
        let t0 = Instant::now();
        clock.callback(u32::MAX - 5);
        clock.step(true, true, t0);
        clock.callback(10);
        let dt = clock.step(true, true, t0 + ms(16)).unwrap();
        assert!((dt - 0.016).abs() < 1e-6);
    }

    #[test]
    fn without_pending_frames_the_timer_ticks_by_wall_clock() {
        let mut clock = FrameClock::default();
        let t0 = Instant::now();
        clock.step(true, false, t0);
        let dt = clock.step(true, false, t0 + ms(16)).unwrap();
        assert!((dt - 0.016).abs() < 1e-6);
    }

    #[test]
    fn long_gaps_are_clamped() {
        let mut clock = FrameClock::default();
        let t0 = Instant::now();
        clock.callback(0);
        clock.step(true, true, t0);
        clock.callback(5000);
        assert_eq!(clock.step(true, true, t0 + ms(5000)), Some(MAX_STEP));
    }

    #[test]
    fn stopping_resets_the_clock() {
        let mut clock = FrameClock::default();
        let t0 = Instant::now();
        clock.callback(0);
        clock.step(true, true, t0);
        clock.step(false, true, t0 + ms(10));
        clock.callback(500);
        assert_eq!(clock.step(true, true, t0 + ms(500)), Some(0.0));
    }
}
