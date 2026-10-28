use std::time::{SystemTime, UNIX_EPOCH};

use chrono::{DateTime, Local};

/// Source of the current time, replaceable in tests.
pub trait Clock {
    /// The current local time.
    fn now(&self) -> DateTime<Local>;

    /// Unix time in milliseconds.
    fn unix_ms(&self) -> f64 {
        self.now().timestamp_millis() as f64
    }

    /// Unix time in whole seconds.
    fn unix_secs(&self) -> i64 {
        self.now().timestamp()
    }

    /// Unix time in seconds with a fractional part.
    fn unix_secs_f64(&self) -> f64 {
        self.unix_ms() / 1000.0
    }
}

/// The clock of the host system.
#[derive(Debug, Clone, Copy, Default)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> DateTime<Local> {
        Local::now()
    }

    fn unix_ms(&self) -> f64 {
        SystemTime::now().duration_since(UNIX_EPOCH).map(|elapsed| elapsed.as_secs_f64() * 1000.0).unwrap_or(0.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_system_clock_agrees_with_chrono() {
        let clock = SystemClock;
        let secs = clock.unix_secs();
        assert!((clock.unix_ms() / 1000.0 - secs as f64).abs() < 2.0);
        assert!((clock.unix_secs_f64() - secs as f64).abs() < 2.0);
        assert!((clock.now().timestamp() - secs).abs() < 2);
    }
}
