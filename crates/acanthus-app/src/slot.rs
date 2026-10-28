use std::cell::{Cell, RefCell};
use std::rc::{Rc, Weak};

enum Held<T> {
    Empty,
    Strong(Rc<T>),
    Weak(Weak<T>),
}

/// Lazily started, shared service. The first caller starts it, later callers get the same `Rc`.
///
/// A [`Slot::new`] slot keeps the service for the life of the slot. A [`Slot::demand`] slot only
/// weakly references it, so the service stops when the last consumer drops its handle and starts
/// again on the next request.
pub struct Slot<T> {
    held: RefCell<Held<T>>,
    keep: bool,
    starts: Cell<u32>,
}

impl<T> Slot<T> {
    /// A slot that keeps the service once started.
    pub fn new() -> Slot<T> {
        Slot { held: RefCell::new(Held::Empty), keep: true, starts: Cell::new(0) }
    }

    /// A slot that lets the service stop when the last handle is dropped.
    pub fn demand() -> Slot<T> {
        Slot { held: RefCell::new(Held::Empty), keep: false, starts: Cell::new(0) }
    }

    /// The running service, if any.
    pub fn get(&self) -> Option<Rc<T>> {
        match &*self.held.borrow() {
            Held::Empty => None,
            Held::Strong(service) => Some(service.clone()),
            Held::Weak(service) => service.upgrade(),
        }
    }

    /// Returns the running service or starts it with `start`. A failed start leaves the slot empty.
    /// `start` may request other slots.
    pub fn get_or_start<E>(&self, start: impl FnOnce() -> Result<T, E>) -> Result<Rc<T>, E> {
        if let Some(service) = self.get() {
            return Ok(service);
        }
        let service = Rc::new(start()?);
        self.starts.set(self.starts.get() + 1);
        *self.held.borrow_mut() =
            if self.keep { Held::Strong(service.clone()) } else { Held::Weak(Rc::downgrade(&service)) };
        Ok(service)
    }

    /// Like [`Slot::get_or_start`] for services that cannot fail to start.
    pub fn get_or_init(&self, start: impl FnOnce() -> T) -> Rc<T> {
        match self.get_or_start(|| Ok::<T, std::convert::Infallible>(start())) {
            Ok(service) => service,
            Err(never) => match never {},
        }
    }

    /// How many times the service was started.
    pub fn starts(&self) -> u32 {
        self.starts.get()
    }

    /// Drops the slot's own reference; consumers still holding a handle keep the service alive.
    pub fn release(&self) {
        *self.held.borrow_mut() = Held::Empty;
    }
}

impl<T> Default for Slot<T> {
    fn default() -> Slot<T> {
        Slot::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn consumers_share_one_service() {
        let slot = Slot::new();
        let first = slot.get_or_init(|| 5_u32);
        let second = slot.get_or_init(|| 9_u32);
        assert!(Rc::ptr_eq(&first, &second));
        assert_eq!(*second, 5);
        assert_eq!(slot.starts(), 1);
    }

    #[test]
    fn a_failed_start_is_retried() {
        let slot: Slot<u32> = Slot::new();
        assert_eq!(slot.get_or_start(|| Err("down")).unwrap_err(), "down");
        assert!(slot.get().is_none());
        assert_eq!(*slot.get_or_start(|| Ok::<_, &str>(3)).unwrap(), 3);
        assert_eq!(slot.starts(), 1);
    }

    #[test]
    fn a_demand_slot_stops_with_the_last_handle() {
        let slot = Slot::demand();
        let first = slot.get_or_init(|| 1_u32);
        let again = slot.get_or_init(|| 2_u32);
        assert!(Rc::ptr_eq(&first, &again));
        drop((first, again));
        assert!(slot.get().is_none());
        assert_eq!(*slot.get_or_init(|| 3_u32), 3);
        assert_eq!(slot.starts(), 2);
    }

    #[test]
    fn a_kept_slot_outlives_its_consumers() {
        let slot = Slot::new();
        drop(slot.get_or_init(|| 1_u32));
        assert!(slot.get().is_some());
        slot.release();
        assert!(slot.get().is_none());
    }

    #[test]
    fn a_start_may_request_another_slot() {
        let inner = Slot::new();
        let outer = Slot::new();
        let value = outer.get_or_init(|| *inner.get_or_init(|| 4_u32) + 1);
        assert_eq!(*value, 5);
    }
}
