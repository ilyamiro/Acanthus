use std::cell::{Cell, RefCell};
use std::collections::VecDeque;
use std::fmt;
use std::rc::Rc;

use slotmap::{SlotMap, new_key_type};

new_key_type! {
    struct EffectId;
}

const MAX_FLUSH_RUNS: usize = 100_000;

struct EffectSlot {
    run: Option<Box<dyn FnMut()>>,
    queued: bool,
}

#[derive(Default)]
struct Runtime {
    effects: RefCell<SlotMap<EffectId, EffectSlot>>,
    observer: Cell<Option<EffectId>>,
    queue: RefCell<VecDeque<EffectId>>,
    batch_depth: Cell<u32>,
    flushing: Cell<bool>,
    epoch: Cell<u64>,
}

thread_local! {
    static RT: Runtime = Runtime::default();
}

/// A counter that increases on every signal write on this thread; equal values mean no signal changed in between.
pub fn epoch() -> u64 {
    RT.with(|rt| rt.epoch.get())
}

/// Runs `f` without registering signal reads as dependencies of the running effect.
pub fn untrack<R>(f: impl FnOnce() -> R) -> R {
    let prev = RT.with(|rt| rt.observer.replace(None));
    let out = f();
    RT.with(|rt| rt.observer.set(prev));
    out
}

/// Runs `f` and defers effect re-runs until the outermost batch ends, so several writes trigger one run.
pub fn batch<R>(f: impl FnOnce() -> R) -> R {
    RT.with(|rt| rt.batch_depth.set(rt.batch_depth.get() + 1));
    let out = f();
    let outermost = RT.with(|rt| {
        let depth = rt.batch_depth.get() - 1;
        rt.batch_depth.set(depth);
        depth == 0
    });
    if outermost {
        flush();
    }
    out
}

fn enqueue(id: EffectId) {
    RT.with(|rt| {
        if let Some(slot) = rt.effects.borrow_mut().get_mut(id)
            && !slot.queued
        {
            slot.queued = true;
            rt.queue.borrow_mut().push_back(id);
        }
    });
}

fn flush() {
    let start = RT.with(|rt| {
        if rt.flushing.get() || rt.batch_depth.get() > 0 {
            false
        } else {
            rt.flushing.set(true);
            true
        }
    });
    if !start {
        return;
    }

    struct Reset;
    impl Drop for Reset {
        fn drop(&mut self) {
            RT.with(|rt| rt.flushing.set(false));
        }
    }
    let _reset = Reset;

    let mut runs = 0usize;
    while let Some(id) = RT.with(|rt| rt.queue.borrow_mut().pop_front()) {
        runs += 1;
        if runs > MAX_FLUSH_RUNS {
            tracing::error!("binding loop detected; dropping pending updates");
            RT.with(|rt| {
                let mut effects = rt.effects.borrow_mut();
                for id in rt.queue.borrow_mut().drain(..) {
                    if let Some(slot) = effects.get_mut(id) {
                        slot.queued = false;
                    }
                }
            });
            break;
        }
        run_effect(id);
    }
}

fn run_effect(id: EffectId) {
    let taken = RT.with(|rt| {
        let mut effects = rt.effects.borrow_mut();
        let slot = effects.get_mut(id)?;
        slot.queued = false;
        slot.run.take()
    });
    let Some(mut f) = taken else { return };

    let prev = RT.with(|rt| rt.observer.replace(Some(id)));
    f();
    RT.with(|rt| rt.observer.set(prev));

    let orphan = RT.with(|rt| match rt.effects.borrow_mut().get_mut(id) {
        Some(slot) => {
            slot.run = Some(f);
            None
        }
        None => Some(f),
    });
    drop(orphan);
}

/// A side effect that runs once at creation and again, synchronously, whenever a signal it read changes. Dropping it disposes it.
#[must_use = "dropping an Effect disposes it immediately"]
pub struct Effect(EffectId);

impl Effect {
    /// Creates the effect and runs `f` immediately, recording the signals it reads.
    pub fn new(f: impl FnMut() + 'static) -> Self {
        let id = RT.with(|rt| rt.effects.borrow_mut().insert(EffectSlot { run: Some(Box::new(f)), queued: false }));
        run_effect(id);
        Effect(id)
    }

    /// Keeps the effect alive for the rest of the program instead of disposing it on drop.
    pub fn leak(self) {
        std::mem::forget(self);
    }
}

impl Drop for Effect {
    fn drop(&mut self) {
        let slot = RT.with(|rt| rt.effects.borrow_mut().remove(self.0));
        drop(slot);
    }
}

impl fmt::Debug for Effect {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("Effect").field(&self.0).finish()
    }
}

/// Dirty flag fed by the signals read inside [`Tracker::track`]; lets a consumer such as a scene redraw only when something it read changed.
pub struct Tracker {
    id: EffectId,
    dirty: Rc<Cell<bool>>,
}

impl Tracker {
    /// Creates a tracker that starts dirty.
    pub fn new() -> Self {
        let dirty = Rc::new(Cell::new(true));
        let flag = dirty.clone();
        let id = RT.with(|rt| {
            rt.effects.borrow_mut().insert(EffectSlot { run: Some(Box::new(move || flag.set(true))), queued: false })
        });
        Tracker { id, dirty }
    }

    /// Whether a tracked signal changed (or [`Tracker::mark_dirty`] was called) since the last [`Tracker::track`].
    pub fn is_dirty(&self) -> bool {
        self.dirty.get()
    }

    /// Forces the dirty flag on.
    pub fn mark_dirty(&self) {
        self.dirty.set(true);
    }

    /// Runs `f`, clears the dirty flag and subscribes the tracker to every signal `f` reads.
    pub fn track<R>(&self, f: impl FnOnce() -> R) -> R {
        self.dirty.set(false);
        let prev = RT.with(|rt| rt.observer.replace(Some(self.id)));
        let out = f();
        RT.with(|rt| rt.observer.set(prev));
        out
    }
}

impl Default for Tracker {
    fn default() -> Self {
        Tracker::new()
    }
}

impl Drop for Tracker {
    fn drop(&mut self) {
        let slot = RT.with(|rt| rt.effects.borrow_mut().remove(self.id));
        drop(slot);
    }
}

struct SignalInner<T> {
    value: RefCell<T>,
    subscribers: RefCell<Vec<EffectId>>,
}

/// Observable shared value. Clones share the same value; reading inside an [`Effect`] subscribes it.
pub struct Signal<T>(Rc<SignalInner<T>>);

impl<T> Clone for Signal<T> {
    fn clone(&self) -> Self {
        Signal(self.0.clone())
    }
}

impl<T: Default + 'static> Default for Signal<T> {
    fn default() -> Self {
        Signal::new(T::default())
    }
}

impl<T: fmt::Debug> fmt::Debug for Signal<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("Signal").field(&*self.0.value.borrow()).finish()
    }
}

impl<T: 'static> Signal<T> {
    /// Creates a signal holding `value`.
    pub fn new(value: T) -> Self {
        Signal(Rc::new(SignalInner { value: RefCell::new(value), subscribers: RefCell::new(Vec::new()) }))
    }

    fn track(&self) {
        if let Some(id) = RT.with(|rt| rt.observer.get()) {
            let mut subs = self.0.subscribers.borrow_mut();
            if !subs.contains(&id) {
                subs.push(id);
            }
        }
    }

    fn notify(&self) {
        RT.with(|rt| rt.epoch.set(rt.epoch.get() + 1));
        let subs = std::mem::take(&mut *self.0.subscribers.borrow_mut());
        for id in subs {
            enqueue(id);
        }
        flush();
    }

    /// Reads the value by reference and tracks the read.
    pub fn with<R>(&self, f: impl FnOnce(&T) -> R) -> R {
        self.track();
        f(&self.0.value.borrow())
    }

    /// Reads the value by reference without tracking.
    pub fn with_untracked<R>(&self, f: impl FnOnce(&T) -> R) -> R {
        f(&self.0.value.borrow())
    }

    /// Like [`Signal::with_untracked`] but returns `None` instead of panicking when the value is being written.
    pub fn try_with_untracked<R>(&self, f: impl FnOnce(&T) -> R) -> Option<R> {
        self.0.value.try_borrow().ok().map(|value| f(&value))
    }

    /// Edits the value in place and notifies subscribers. Ignored (and logged) when the value is already borrowed.
    pub fn update(&self, f: impl FnOnce(&mut T)) {
        match self.0.value.try_borrow_mut() {
            Ok(mut value) => f(&mut value),
            Err(_) => {
                tracing::error!("Signal::update ignored: the signal is already borrowed");
                return;
            }
        }
        self.notify();
    }

    /// Replaces the value and notifies subscribers even when it is equal to the old one.
    pub fn set_forced(&self, value: T) {
        match self.0.value.try_borrow_mut() {
            Ok(mut slot) => *slot = value,
            Err(_) => {
                tracing::error!("Signal write ignored: the signal is already borrowed");
                return;
            }
        }
        self.notify();
    }

    /// Whether both handles share the same value.
    pub fn ptr_eq(&self, other: &Signal<T>) -> bool {
        Rc::ptr_eq(&self.0, &other.0)
    }
}

impl<T: Clone + 'static> Signal<T> {
    /// A clone of the value; tracks the read.
    pub fn get(&self) -> T {
        self.with(T::clone)
    }

    /// A clone of the value without tracking.
    pub fn get_untracked(&self) -> T {
        self.with_untracked(T::clone)
    }

    /// A clone of the value, or `None` when it is being written.
    pub fn try_get_untracked(&self) -> Option<T> {
        self.try_with_untracked(T::clone)
    }
}

impl<T: PartialEq + 'static> Signal<T> {
    /// Replaces the value and notifies subscribers only when it differs from the current one.
    pub fn set(&self, value: T) {
        if self.0.value.try_borrow().is_ok_and(|current| *current == value) {
            return;
        }
        self.set_forced(value);
    }

    /// Creates an effect that writes `f()` into this signal whenever the signals `f` reads change.
    pub fn bind(&self, mut f: impl FnMut() -> T + 'static) -> Effect {
        let target = self.clone();
        Effect::new(move || {
            let value = f();
            untrack(|| target.set(value));
        })
    }
}

/// A signal derived from other signals; recomputed when its inputs change and notifying only when the result differs.
pub struct Memo<T> {
    /// The derived value.
    pub signal: Signal<T>,
    effect: Effect,
}

impl<T: PartialEq + 'static> Memo<T> {
    /// Computes `f` once and keeps the value up to date.
    pub fn new(mut f: impl FnMut() -> T + 'static) -> Self {
        let signal = Signal::new(f());
        let effect = signal.bind(f);
        Memo { signal, effect }
    }

    /// Separates the signal from the effect that keeps it current, so the effect can be stored elsewhere.
    pub fn split(self) -> (Signal<T>, Effect) {
        (self.signal, self.effect)
    }
}

impl<T: Clone + 'static> Memo<T> {
    /// A clone of the derived value; tracks the read.
    pub fn get(&self) -> T {
        self.signal.get()
    }
}

/// A bag that owns effects and other values so they all drop together.
#[derive(Default)]
pub struct Scope(Vec<Box<dyn std::any::Any>>);

impl Scope {
    /// Stores a value (typically an [`Effect`]) until the scope drops.
    pub fn add(&mut self, value: impl std::any::Any) {
        self.0.push(Box::new(value));
    }

    /// Creates a memo from `f`, keeps its effect in the scope and returns the signal.
    pub fn derive<T: PartialEq + 'static>(&mut self, f: impl FnMut() -> T + 'static) -> Signal<T> {
        let (signal, effect) = Memo::new(f).split();
        self.add(effect);
        signal
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn memo_follows_its_sources_and_scope_drops_the_binding() {
        let a = Signal::new(1);
        let memo = {
            let a = a.clone();
            Memo::new(move || a.get() * 2)
        };
        a.set(4);
        assert_eq!(memo.get(), 8);
        let mut scope = Scope::default();
        let doubled = {
            let a = a.clone();
            scope.derive(move || a.get() + 1)
        };
        a.set(5);
        assert_eq!(doubled.get_untracked(), 6);
        drop(scope);
        a.set(9);
        assert_eq!(doubled.get_untracked(), 6);
    }

    #[test]
    fn writing_a_signal_inside_its_own_update_is_ignored_instead_of_panicking() {
        let a = Signal::new(1);
        let seen = Rc::new(Cell::new(0));
        let _e = {
            let (a, seen) = (a.clone(), seen.clone());
            Effect::new(move || seen.set(a.get()))
        };
        let inner = a.clone();
        a.update(move |v| {
            *v = 5;
            inner.set(9);
            inner.update(|v| *v = 11);
            inner.set_forced(13);
        });
        assert_eq!(a.get_untracked(), 5);
        assert_eq!(seen.get(), 5);
        a.set(6);
        assert_eq!(seen.get(), 6);
    }

    #[test]
    fn effect_reruns_on_dependency_change() {
        let a = Signal::new(1);
        let seen = Rc::new(Cell::new(0));
        let s = seen.clone();
        let a2 = a.clone();
        let _e = Effect::new(move || s.set(a2.get() * 10));
        assert_eq!(seen.get(), 10);
        a.set(2);
        assert_eq!(seen.get(), 20);
    }

    #[test]
    fn equal_set_does_not_rerun() {
        let a = Signal::new(5);
        let runs = Rc::new(Cell::new(0));
        let r = runs.clone();
        let a2 = a.clone();
        let _e = Effect::new(move || {
            a2.get();
            r.set(r.get() + 1);
        });
        a.set(5);
        assert_eq!(runs.get(), 1);
        a.set_forced(5);
        assert_eq!(runs.get(), 2);
    }

    #[test]
    fn binding_chain_is_synchronous() {
        let width = Signal::new(100.0_f32);
        let half = Signal::new(0.0_f32);
        let quarter = Signal::new(0.0_f32);
        let w = width.clone();
        let _b1 = half.bind(move || w.get() / 2.0);
        let h = half.clone();
        let _b2 = quarter.bind(move || h.get() / 2.0);
        assert_eq!(quarter.get(), 25.0);
        width.set(200.0);
        assert_eq!(quarter.get(), 50.0);
    }

    #[test]
    fn batch_coalesces_runs() {
        let a = Signal::new(0);
        let b = Signal::new(0);
        let runs = Rc::new(Cell::new(0));
        let (a2, b2, r) = (a.clone(), b.clone(), runs.clone());
        let _e = Effect::new(move || {
            a2.get();
            b2.get();
            r.set(r.get() + 1);
        });
        batch(|| {
            a.set(1);
            b.set(1);
        });
        assert_eq!(runs.get(), 2);
    }

    #[test]
    fn dropped_effect_stops_running() {
        let a = Signal::new(0);
        let runs = Rc::new(Cell::new(0));
        let (a2, r) = (a.clone(), runs.clone());
        let e = Effect::new(move || {
            a2.get();
            r.set(r.get() + 1);
        });
        drop(e);
        a.set(1);
        assert_eq!(runs.get(), 1);
    }

    #[test]
    fn untracked_reads_do_not_subscribe() {
        let a = Signal::new(0);
        let runs = Rc::new(Cell::new(0));
        let (a2, r) = (a.clone(), runs.clone());
        let _e = Effect::new(move || {
            untrack(|| a2.get());
            r.set(r.get() + 1);
        });
        a.set(1);
        assert_eq!(runs.get(), 1);
    }

    #[test]
    fn binding_loop_terminates() {
        let a = Signal::new(0u64);
        let b = Signal::new(0u64);
        let a2 = a.clone();
        let _ab = b.bind(move || a2.get() + 1);
        let b2 = b.clone();
        let _ba = a.bind(move || b2.get() + 1);
        assert!(a.get_untracked() > 0);
    }

    #[test]
    fn epoch_advances_on_change() {
        let a = Signal::new(0);
        let before = epoch();
        a.set(1);
        assert!(epoch() > before);
        let mid = epoch();
        a.set(1);
        assert_eq!(epoch(), mid);
    }

    #[test]
    fn writes_inside_own_update_are_ignored() {
        let a = Signal::new(1);
        let a2 = a.clone();
        a.update(move |v| {
            a2.set(5);
            a2.update(|inner| *inner = 9);
            *v = 2;
        });
        assert_eq!(a.get_untracked(), 2);
    }

    #[test]
    fn try_get_is_none_while_updating() {
        let a = Signal::new(1);
        let a2 = a.clone();
        let seen = Rc::new(Cell::new(Some(0)));
        let s = seen.clone();
        a.update(move |v| {
            s.set(a2.try_get_untracked());
            *v = 3;
        });
        assert_eq!(seen.get(), None);
        assert_eq!(a.try_get_untracked(), Some(3));
    }
}
