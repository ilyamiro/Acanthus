use std::time::{Duration, Instant};

const MAX_FAILURES: u32 = 5;
const RETRY_BASE: Duration = Duration::from_millis(100);

#[derive(Debug, Clone)]
pub(crate) struct Pacer {
    live: bool,
    interval: Duration,
    failures: u32,
}

impl Pacer {
    pub fn new(live: bool, max_fps: u32) -> Pacer {
        Pacer { live, interval: Duration::from_secs_f64(1.0 / f64::from(max_fps.clamp(1, 240))), failures: 0 }
    }

    pub fn after_frame(&mut self, started: Instant, now: Instant) -> Option<Duration> {
        self.failures = 0;
        self.live.then(|| self.interval.saturating_sub(now.saturating_duration_since(started)))
    }

    pub fn after_failure(&mut self) -> Option<Duration> {
        self.failures += 1;
        (self.failures <= MAX_FAILURES).then(|| RETRY_BASE * 2u32.pow(self.failures - 1))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_one_shot_capture_stops_after_its_frame() {
        let now = Instant::now();
        assert_eq!(Pacer::new(false, 30).after_frame(now, now), None);
    }

    #[test]
    fn live_capture_waits_out_the_rest_of_the_frame_interval() {
        let mut pacer = Pacer::new(true, 50);
        let start = Instant::now();
        assert_eq!(pacer.after_frame(start, start + Duration::from_millis(5)), Some(Duration::from_millis(15)));
        assert_eq!(pacer.after_frame(start, start + Duration::from_millis(40)), Some(Duration::ZERO));
    }

    #[test]
    fn failures_back_off_then_give_up_and_success_resets() {
        let mut pacer = Pacer::new(true, 30);
        let delays: Vec<_> = std::iter::from_fn(|| pacer.after_failure()).collect();
        assert_eq!(delays.len(), MAX_FAILURES as usize);
        assert_eq!(delays[0], Duration::from_millis(100));
        assert_eq!(delays[4], Duration::from_millis(1600));
        let now = Instant::now();
        pacer.after_frame(now, now);
        assert_eq!(pacer.after_failure(), Some(Duration::from_millis(100)));
    }

    #[test]
    fn the_frame_rate_is_clamped() {
        let mut pacer = Pacer::new(true, 0);
        let now = Instant::now();
        assert_eq!(pacer.after_frame(now, now), Some(Duration::from_secs(1)));
    }
}
