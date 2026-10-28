use std::time::{Duration, Instant};

use calloop::timer::{TimeoutAction, Timer as CalloopTimer};
use calloop::{LoopHandle, RegistrationToken};

use crate::error::IoError;
use crate::weak::WeakHandle;

/// Handle to a timer; dropping it cancels the timer.
pub struct TimerHandle {
    remove: Option<Box<dyn FnOnce()>>,
}

impl TimerHandle {
    /// Handle that runs `remove` when cancelled or dropped.
    pub fn from_fn(remove: impl FnOnce() + 'static) -> TimerHandle {
        TimerHandle { remove: Some(Box::new(remove)) }
    }

    /// Cancels the timer.
    pub fn cancel(mut self) {
        self.run_remove();
    }

    /// Keeps the timer running after the handle is gone.
    pub fn detach(mut self) {
        self.remove = None;
    }

    fn run_remove(&mut self) {
        if let Some(remove) = self.remove.take() {
            remove();
        }
    }
}

impl Drop for TimerHandle {
    fn drop(&mut self) {
        self.run_remove();
    }
}

impl std::fmt::Debug for TimerHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("TimerHandle")
    }
}

fn handle_for<D: 'static>(handle: &LoopHandle<'static, D>, token: RegistrationToken) -> TimerHandle {
    let weak = WeakHandle::new(handle);
    TimerHandle { remove: Some(Box::new(move || weak.remove(token))) }
}

/// Runs `f` once after `delay`.
pub fn after<D: 'static>(
    handle: &LoopHandle<'static, D>,
    delay: Duration,
    f: impl FnOnce() + 'static,
) -> Result<TimerHandle, IoError> {
    let mut f = Some(f);
    let token = handle.insert_source(CalloopTimer::from_duration(delay), move |_, _, _| {
        if let Some(f) = f.take() {
            f();
        }
        TimeoutAction::Drop
    })?;
    Ok(handle_for(handle, token))
}

/// Runs `f` every `interval`, without drift.
pub fn every<D: 'static>(
    handle: &LoopHandle<'static, D>,
    interval: Duration,
    mut f: impl FnMut() + 'static,
) -> Result<TimerHandle, IoError> {
    let token = handle.insert_source(CalloopTimer::from_duration(interval), move |deadline, _, _| {
        f();
        TimeoutAction::ToInstant(deadline.max(Instant::now()) + interval)
    })?;
    Ok(handle_for(handle, token))
}

/// Runs `f` after `first`; each returned duration schedules the next run and `None` stops the timer.
pub fn dynamic<D: 'static>(
    handle: &LoopHandle<'static, D>,
    first: Duration,
    mut f: impl FnMut() -> Option<Duration> + 'static,
) -> Result<TimerHandle, IoError> {
    let token = handle.insert_source(CalloopTimer::from_duration(first), move |_, _, _| match f() {
        Some(next) => TimeoutAction::ToDuration(next),
        None => TimeoutAction::Drop,
    })?;
    Ok(handle_for(handle, token))
}

/// Function that starts a dynamic timer, usable without naming the loop data type.
pub type Scheduler = std::rc::Rc<dyn Fn(Duration, Box<dyn FnMut() -> Option<Duration>>) -> Option<TimerHandle>>;

/// A [`Scheduler`] bound weakly to the event loop; it returns `None` once the loop is gone.
pub fn scheduler<D: 'static>(handle: &LoopHandle<'static, D>) -> Scheduler {
    let weak = WeakHandle::new(handle);
    std::rc::Rc::new(move |first, f| dynamic(&weak.upgrade()?, first, f).ok())
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;
    use std::rc::Rc;

    use calloop::EventLoop;

    use super::*;

    fn pump(event_loop: &mut EventLoop<'static, ()>, ms: u64) {
        let end = Instant::now() + Duration::from_millis(ms);
        while Instant::now() < end {
            event_loop.dispatch(Duration::from_millis(5), &mut ()).unwrap();
        }
    }

    #[test]
    fn after_fires_once_and_can_be_cancelled() {
        let mut event_loop: EventLoop<'static, ()> = EventLoop::try_new().unwrap();
        let hits = Rc::new(Cell::new(0));
        let h = hits.clone();
        let _t = after(&event_loop.handle(), Duration::from_millis(10), move || h.set(h.get() + 1)).unwrap();
        let h = hits.clone();
        let cancelled = after(&event_loop.handle(), Duration::from_millis(10), move || h.set(h.get() + 100)).unwrap();
        cancelled.cancel();
        pump(&mut event_loop, 80);
        assert_eq!(hits.get(), 1);
    }

    #[test]
    fn every_repeats_until_dropped() {
        let mut event_loop: EventLoop<'static, ()> = EventLoop::try_new().unwrap();
        let hits = Rc::new(Cell::new(0));
        let h = hits.clone();
        let t = every(&event_loop.handle(), Duration::from_millis(10), move || h.set(h.get() + 1)).unwrap();
        pump(&mut event_loop, 100);
        assert!(hits.get() >= 3, "{}", hits.get());
        drop(t);
        let seen = hits.get();
        pump(&mut event_loop, 50);
        assert_eq!(hits.get(), seen);
    }

    #[test]
    fn scheduler_runs_dynamic_timers_without_the_loop_type() {
        let mut event_loop: EventLoop<'static, ()> = EventLoop::try_new().unwrap();
        let schedule = scheduler(&event_loop.handle());
        let hits = Rc::new(Cell::new(0));
        let h = hits.clone();
        let handle = schedule(
            Duration::from_millis(5),
            Box::new(move || {
                h.set(h.get() + 1);
                Some(Duration::from_millis(5))
            }),
        );
        pump(&mut event_loop, 60);
        assert!(hits.get() >= 3);
        drop(handle);
        let seen = hits.get();
        pump(&mut event_loop, 40);
        assert_eq!(hits.get(), seen);
    }

    #[test]
    fn dynamic_chooses_next_delay_and_can_stop() {
        let mut event_loop: EventLoop<'static, ()> = EventLoop::try_new().unwrap();
        let hits = Rc::new(Cell::new(0));
        let h = hits.clone();
        let _t = dynamic(&event_loop.handle(), Duration::from_millis(5), move || {
            h.set(h.get() + 1);
            (h.get() < 3).then_some(Duration::from_millis(5))
        })
        .unwrap();
        pump(&mut event_loop, 100);
        assert_eq!(hits.get(), 3);
    }
}
