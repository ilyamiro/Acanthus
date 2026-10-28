use std::time::{Duration, Instant};

#[derive(Debug, Clone)]
pub(crate) struct LazyGate {
    linger: Duration,
    close_at: Option<Instant>,
    mapped: bool,
}

impl LazyGate {
    pub fn new(linger: Duration) -> Self {
        LazyGate { linger, close_at: None, mapped: false }
    }

    pub fn step(&mut self, open: bool, now: Instant) -> bool {
        if open {
            self.close_at = None;
            self.mapped = true;
        } else if self.mapped {
            let at = *self.close_at.get_or_insert(now + self.linger);
            if now >= at {
                self.mapped = false;
                self.close_at = None;
            }
        }
        self.mapped
    }

    pub fn deadline(&self) -> Option<Instant> {
        self.close_at
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const LINGER: Duration = Duration::from_millis(500);

    #[test]
    fn starts_unmapped_and_maps_on_open() {
        let mut gate = LazyGate::new(LINGER);
        let t0 = Instant::now();
        assert!(!gate.step(false, t0));
        assert!(gate.step(true, t0));
    }

    #[test]
    fn lingers_after_close_then_unmaps() {
        let mut gate = LazyGate::new(LINGER);
        let t0 = Instant::now();
        gate.step(true, t0);
        assert!(gate.step(false, t0));
        assert_eq!(gate.deadline(), Some(t0 + LINGER));
        assert!(gate.step(false, t0 + Duration::from_millis(499)));
        assert!(!gate.step(false, t0 + LINGER));
        assert_eq!(gate.deadline(), None);
    }

    #[test]
    fn reopening_cancels_the_pending_unmap() {
        let mut gate = LazyGate::new(LINGER);
        let t0 = Instant::now();
        gate.step(true, t0);
        gate.step(false, t0);
        assert!(gate.step(true, t0 + Duration::from_millis(300)));
        assert_eq!(gate.deadline(), None);
        assert!(gate.step(false, t0 + Duration::from_millis(400)));
        assert_eq!(gate.deadline(), Some(t0 + Duration::from_millis(900)));
    }

    #[test]
    fn closed_gate_has_no_deadline() {
        let mut gate = LazyGate::new(LINGER);
        gate.step(false, Instant::now());
        assert_eq!(gate.deadline(), None);
    }
}
