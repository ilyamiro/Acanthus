use std::cell::{Cell, RefCell};
use std::rc::Rc;

use calloop::{LoopHandle, RegistrationToken};

thread_local! {
    static REMOVING: Cell<bool> = const { Cell::new(false) };
    static DEFERRED: RefCell<Vec<Box<dyn FnOnce()>>> = const { RefCell::new(Vec::new()) };
}

/// A weak reference to a [`LoopHandle`] for callbacks that may outlive the loop.
pub struct WeakHandle<D: 'static>(Rc<dyn Fn() -> Option<LoopHandle<'static, D>>>);

impl<D: 'static> WeakHandle<D> {
    /// Downgrades `handle`.
    pub fn new(handle: &LoopHandle<'static, D>) -> Self {
        let weak = handle.downgrade();
        WeakHandle(Rc::new(move || weak.upgrade()))
    }

    /// The handle, if the loop still exists.
    pub fn upgrade(&self) -> Option<LoopHandle<'static, D>> {
        (self.0)()
    }

    /// Removes an event source; safe to call from inside a callback, the removal is then deferred.
    pub fn remove(&self, token: RegistrationToken) {
        if REMOVING.with(Cell::get) {
            let this = self.clone();
            DEFERRED.with(|d| d.borrow_mut().push(Box::new(move || this.remove(token))));
            return;
        }
        REMOVING.with(|r| r.set(true));
        if let Some(handle) = self.upgrade() {
            handle.remove(token);
        }
        REMOVING.with(|r| r.set(false));
        loop {
            let next = DEFERRED.with(|d| {
                let mut queue = d.borrow_mut();
                (!queue.is_empty()).then(|| queue.remove(0))
            });
            match next {
                Some(job) => job(),
                None => break,
            }
        }
    }
}

impl<D: 'static> Clone for WeakHandle<D> {
    fn clone(&self) -> Self {
        WeakHandle(self.0.clone())
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use calloop::EventLoop;
    use calloop::timer::{TimeoutAction, Timer};

    use super::*;

    struct RemovesOnDrop {
        handle: WeakHandle<()>,
        token: RegistrationToken,
    }

    impl Drop for RemovesOnDrop {
        fn drop(&mut self) {
            self.handle.remove(self.token);
        }
    }

    #[test]
    fn removal_triggered_by_dropping_a_removed_source_does_not_panic() {
        let event_loop: EventLoop<'static, ()> = EventLoop::try_new().unwrap();
        let handle = event_loop.handle();
        let weak = WeakHandle::new(&handle);
        let victim =
            handle.insert_source(Timer::from_duration(Duration::from_secs(60)), |_, _, _| TimeoutAction::Drop).unwrap();
        let guard = RemovesOnDrop { handle: weak.clone(), token: victim };
        let carrier = handle
            .insert_source(Timer::from_duration(Duration::from_secs(60)), move |_, _, _| {
                let _keep = &guard;
                TimeoutAction::Drop
            })
            .unwrap();
        weak.remove(carrier);
    }
}
