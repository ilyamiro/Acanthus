use std::cell::{Cell, RefCell};
use std::fmt::Debug;
use std::rc::{Rc, Weak};
use std::time::Duration;

use acanthus_core::{Effect, Signal, batch, untrack};
use acanthus_io::TimerHandle;

use super::{Ctx, Running, Store, Stored};

pub type FetchDone<D> = Box<dyn FnOnce(Option<D>)>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Policy {
    pub interval: Option<Duration>,
    pub fresh_for: Duration,
    pub retry_after_failure: Duration,
    pub stale_after: Option<Duration>,
    pub debounce: Duration,
    pub gated: bool,
}

impl Default for Policy {
    fn default() -> Policy {
        Policy {
            interval: None,
            fresh_for: Duration::ZERO,
            retry_after_failure: Duration::ZERO,
            stale_after: None,
            debounce: Duration::ZERO,
            gated: false,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FeedStatus {
    Idle,
    Loading,
    Ready,
    Offline,
    Failed,
}

#[derive(Clone)]
pub struct FeedState<D> {
    pub data: Signal<Option<D>>,
    pub status: Signal<FeedStatus>,
    pub loading: Signal<bool>,
    pub fetched_at: Signal<Option<i64>>,
    pub revision: Signal<u64>,
}

impl<D: 'static> FeedState<D> {
    fn new() -> FeedState<D> {
        FeedState {
            data: Signal::new(None),
            status: Signal::new(FeedStatus::Idle),
            loading: Signal::new(false),
            fetched_at: Signal::new(None),
            revision: Signal::new(0),
        }
    }
}

pub trait FeedSpec: 'static {
    type Key: Clone + PartialEq + Debug + 'static;
    type Data: Clone + PartialEq + 'static;
    const NAME: &'static str;

    fn key(&self) -> Option<Self::Key>;
    fn policy(&self) -> Policy;
    fn fetch(&self, ctx: &Ctx, key: &Self::Key, done: FetchDone<Self::Data>) -> Option<Box<dyn Running>>;

    fn fallback(&self, _ctx: &Ctx, _key: &Self::Key) -> Option<Self::Data> {
        None
    }
}

struct Gate {
    count: Cell<usize>,
    wake: RefCell<Option<Rc<dyn Fn()>>>,
}

pub struct Subscription {
    gate: Rc<Gate>,
}

impl Drop for Subscription {
    fn drop(&mut self) {
        self.gate.count.set(self.gate.count.get().saturating_sub(1));
    }
}

struct Inner<S: FeedSpec> {
    spec: S,
    ctx: Ctx,
    store: Box<dyn Store<S::Key, S::Data>>,
    state: FeedState<S::Data>,
    gate: Rc<Gate>,
    started: Cell<bool>,
    key: RefCell<Option<S::Key>>,
    data_key: RefCell<Option<S::Key>>,
    generation: Cell<u64>,
    completed: Cell<u64>,
    running: RefCell<Option<Box<dyn Running>>>,
    interval: RefCell<Option<TimerHandle>>,
    armed_interval: Cell<Option<Duration>>,
    debounce: RefCell<Option<TimerHandle>>,
    retry: RefCell<Option<TimerHandle>>,
}

impl<S: FeedSpec> Inner<S> {
    fn policy(&self) -> Policy {
        untrack(|| self.spec.policy())
    }

    fn on_key(self: &Rc<Self>, key: Option<S::Key>) {
        let first = !self.started.replace(true);
        if first {
            self.load_stored(key.as_ref());
        } else if *self.key.borrow() == key {
            return;
        }
        *self.key.borrow_mut() = key.clone();
        if let Some(handle) = self.debounce.borrow_mut().take() {
            handle.cancel();
        }
        if key.is_none() {
            self.cancel_inflight();
            self.state.loading.set(false);
            if self.state.data.get_untracked().is_none() {
                self.state.status.set(FeedStatus::Idle);
            }
            return;
        }
        if first {
            self.fetch(false);
            return;
        }
        let debounce = self.policy().debounce;
        if debounce.is_zero() {
            self.fetch(true);
            return;
        }
        let weak = Rc::downgrade(self);
        let timer = (self.ctx.schedule)(
            debounce,
            Box::new(move || {
                if let Some(inner) = weak.upgrade() {
                    inner.debounce.borrow_mut().take();
                    inner.fetch(true);
                }
                None
            }),
        );
        *self.debounce.borrow_mut() = timer;
    }

    fn load_stored(&self, key: Option<&S::Key>) {
        let Some(stored) = self.store.load() else { return };
        if key.is_some_and(|key| *key != stored.key) {
            return;
        }
        let policy = self.policy();
        let old = policy.stale_after.is_some_and(|limit| self.store.age().is_none_or(|age| age > limit));
        batch(|| {
            self.state.data.set(Some(stored.data));
            self.state.fetched_at.set(Some(stored.fetched_at));
            self.state.status.set(if old { FeedStatus::Offline } else { FeedStatus::Ready });
            self.state.revision.update(|revision| *revision += 1);
        });
        *self.data_key.borrow_mut() = Some(stored.key);
    }

    fn abort(&self) {
        self.generation.set(self.generation.get() + 1);
        if let Some(running) = self.running.borrow_mut().take() {
            running.kill();
        }
    }

    fn cancel_inflight(&self) {
        self.abort();
        self.completed.set(self.generation.get());
    }

    fn fetch(self: &Rc<Self>, force: bool) {
        let Some(key) = self.key.borrow().clone() else { return };
        let policy = self.policy();
        if policy.gated && self.gate.count.get() == 0 {
            return;
        }
        let in_flight = self.generation.get() != self.completed.get();
        if !force {
            if in_flight {
                return;
            }
            let current = self.data_key.borrow().as_ref() == Some(&key);
            if current && self.store.age().is_some_and(|age| age < policy.fresh_for) {
                return;
            }
        }
        if let Some(handle) = self.retry.borrow_mut().take() {
            handle.cancel();
        }
        self.abort();
        let generation = self.generation.get();
        let has_data = self.state.data.get_untracked().is_some();
        batch(|| {
            self.state.loading.set(force || !has_data);
            if !has_data {
                self.state.status.set(FeedStatus::Loading);
            }
        });
        let weak = Rc::downgrade(self);
        let fetched_key = key.clone();
        let handle = self.spec.fetch(
            &self.ctx,
            &key,
            Box::new(move |result| {
                let Some(inner) = weak.upgrade() else { return };
                if inner.generation.get() != generation {
                    return;
                }
                inner.completed.set(generation);
                inner.running.borrow_mut().take();
                inner.finish(fetched_key, result);
            }),
        );
        if self.generation.get() == generation && self.completed.get() != generation {
            *self.running.borrow_mut() = handle;
        }
    }

    fn finish(self: &Rc<Self>, key: S::Key, result: Option<S::Data>) {
        match result {
            Some(data) => {
                let now = self.ctx.clock.unix_secs();
                self.store.save(&Stored { key: key.clone(), data: data.clone(), fetched_at: now });
                *self.data_key.borrow_mut() = Some(key);
                batch(|| {
                    self.state.data.set(Some(data));
                    self.state.status.set(FeedStatus::Ready);
                    self.state.fetched_at.set(Some(now));
                    self.state.loading.set(false);
                    self.state.revision.update(|revision| *revision += 1);
                });
            }
            None => self.fail(&key),
        }
    }

    fn fail(self: &Rc<Self>, key: &S::Key) {
        tracing::debug!("{}: fetch for {key:?} failed", S::NAME);
        let policy = self.policy();
        let has_data = self.state.data.get_untracked().is_some();
        let stale = !has_data || policy.stale_after.is_some_and(|limit| self.store.age().is_none_or(|age| age > limit));
        batch(|| {
            self.state.loading.set(false);
            if stale {
                match self.spec.fallback(&self.ctx, key) {
                    Some(fallback) => {
                        let now = self.ctx.clock.unix_secs();
                        self.store.save(&Stored { key: key.clone(), data: fallback.clone(), fetched_at: now });
                        *self.data_key.borrow_mut() = Some(key.clone());
                        self.state.data.set(Some(fallback));
                        self.state.status.set(FeedStatus::Offline);
                        self.state.revision.update(|revision| *revision += 1);
                    }
                    None => self.state.status.set(if has_data { FeedStatus::Offline } else { FeedStatus::Failed }),
                }
            }
        });
        if policy.retry_after_failure.is_zero() {
            return;
        }
        let weak = Rc::downgrade(self);
        let timer = (self.ctx.schedule)(
            policy.retry_after_failure,
            Box::new(move || {
                if let Some(inner) = weak.upgrade() {
                    inner.retry.borrow_mut().take();
                    inner.fetch(false);
                }
                None
            }),
        );
        *self.retry.borrow_mut() = timer;
    }

    fn apply_policy(self: &Rc<Self>, policy: Policy) {
        if self.armed_interval.get() == policy.interval
            && (self.interval.borrow().is_some() || policy.interval.is_none())
        {
            return;
        }
        self.armed_interval.set(policy.interval);
        if let Some(handle) = self.interval.borrow_mut().take() {
            handle.cancel();
        }
        let Some(period) = policy.interval.filter(|period| !period.is_zero()) else { return };
        let weak = Rc::downgrade(self);
        let timer = (self.ctx.schedule)(
            period,
            Box::new(move || {
                let inner = weak.upgrade()?;
                inner.fetch(false);
                Some(period)
            }),
        );
        *self.interval.borrow_mut() = timer;
    }
}

impl<S: FeedSpec> Drop for Inner<S> {
    fn drop(&mut self) {
        if let Some(running) = self.running.borrow_mut().take() {
            running.kill();
        }
    }
}

pub struct Feed<S: FeedSpec> {
    inner: Rc<Inner<S>>,
    _key_effect: Effect,
    _policy_effect: Effect,
}

impl<S: FeedSpec> Feed<S> {
    pub fn new(ctx: &Ctx, spec: S, store: Box<dyn Store<S::Key, S::Data>>) -> Feed<S> {
        let inner = Rc::new(Inner {
            spec,
            ctx: ctx.clone(),
            store,
            state: FeedState::new(),
            gate: Rc::new(Gate { count: Cell::new(0), wake: RefCell::new(None) }),
            started: Cell::new(false),
            key: RefCell::new(None),
            data_key: RefCell::new(None),
            generation: Cell::new(0),
            completed: Cell::new(0),
            running: RefCell::new(None),
            interval: RefCell::new(None),
            armed_interval: Cell::new(None),
            debounce: RefCell::new(None),
            retry: RefCell::new(None),
        });
        let weak = Rc::downgrade(&inner);
        let wake: Rc<dyn Fn()> = Rc::new(move || {
            if let Some(inner) = weak.upgrade() {
                inner.fetch(false);
            }
        });
        *inner.gate.wake.borrow_mut() = Some(wake);
        let weak: Weak<Inner<S>> = Rc::downgrade(&inner);
        let key_effect = Effect::new(move || {
            let Some(inner) = weak.upgrade() else { return };
            let key = inner.spec.key();
            untrack(|| inner.on_key(key));
        });
        let weak: Weak<Inner<S>> = Rc::downgrade(&inner);
        let policy_effect = Effect::new(move || {
            let Some(inner) = weak.upgrade() else { return };
            let policy = inner.spec.policy();
            untrack(|| inner.apply_policy(policy));
        });
        Feed { inner, _key_effect: key_effect, _policy_effect: policy_effect }
    }

    pub fn state(&self) -> &FeedState<S::Data> {
        &self.inner.state
    }

    pub fn spec(&self) -> &S {
        &self.inner.spec
    }

    pub fn refresh(&self, force: bool) {
        self.inner.fetch(force);
    }

    pub fn cancel(&self) {
        let inner = &self.inner;
        for slot in [&inner.debounce, &inner.retry] {
            if let Some(handle) = slot.borrow_mut().take() {
                handle.cancel();
            }
        }
        inner.cancel_inflight();
        inner.state.loading.set(false);
    }

    pub fn subscribe(&self) -> Subscription {
        let gate = self.inner.gate.clone();
        let previous = gate.count.get();
        gate.count.set(previous + 1);
        if previous == 0 {
            let wake = gate.wake.borrow().clone();
            if let Some(wake) = wake {
                wake();
            }
        }
        Subscription { gate }
    }
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;

    use super::*;
    use crate::kit::testing::TestCtx;
    use crate::kit::{MemoryStore, Store};

    type Pending = Rc<RefCell<Vec<(String, Option<FetchDone<u32>>)>>>;

    struct Toy {
        key: Signal<Option<String>>,
        noise: Signal<u32>,
        policy: Signal<Policy>,
        pending: Pending,
        killed: Rc<Cell<usize>>,
        fallback: Option<u32>,
    }

    struct Handle(Rc<Cell<usize>>);

    impl Running for Handle {
        fn pid(&self) -> Option<u32> {
            None
        }

        fn kill(&self) {
            self.0.set(self.0.get() + 1);
        }
    }

    impl FeedSpec for Toy {
        type Key = String;
        type Data = u32;
        const NAME: &'static str = "toy";

        fn key(&self) -> Option<String> {
            self.noise.get();
            self.key.get()
        }

        fn policy(&self) -> Policy {
            self.policy.get()
        }

        fn fetch(&self, _ctx: &Ctx, key: &String, done: FetchDone<u32>) -> Option<Box<dyn Running>> {
            self.pending.borrow_mut().push((key.clone(), Some(done)));
            Some(Box::new(Handle(self.killed.clone())))
        }

        fn fallback(&self, _ctx: &Ctx, _key: &String) -> Option<u32> {
            self.fallback
        }
    }

    struct Rig {
        feed: Feed<Toy>,
        test: TestCtx,
        key: Signal<Option<String>>,
        noise: Signal<u32>,
        policy: Signal<Policy>,
        pending: Pending,
        killed: Rc<Cell<usize>>,
        store: Rc<MemoryStore<String, u32>>,
        _dir: tempfile::TempDir,
    }

    struct Shared(Rc<MemoryStore<String, u32>>);

    impl Store<String, u32> for Shared {
        fn load(&self) -> Option<Stored<String, u32>> {
            self.0.load()
        }

        fn save(&self, stored: &Stored<String, u32>) {
            self.0.save(stored);
        }

        fn age(&self) -> Option<Duration> {
            self.0.age()
        }

        fn touch(&self) {
            self.0.touch();
        }
    }

    fn secs(n: u64) -> Duration {
        Duration::from_secs(n)
    }

    fn base_policy() -> Policy {
        Policy {
            fresh_for: secs(900),
            retry_after_failure: secs(3600),
            stale_after: Some(secs(86400)),
            ..Policy::default()
        }
    }

    fn rig_with(key: Option<&str>, policy: Policy, store: MemoryStore<String, u32>, fallback: Option<u32>) -> Rig {
        let dir = tempfile::tempdir().unwrap();
        let test = TestCtx::new(dir.path());
        let key = Signal::new(key.map(str::to_owned));
        let noise = Signal::new(0);
        let policy = Signal::new(policy);
        let pending: Pending = Rc::default();
        let killed = Rc::new(Cell::new(0));
        let store = Rc::new(store);
        let spec = Toy {
            key: key.clone(),
            noise: noise.clone(),
            policy: policy.clone(),
            pending: pending.clone(),
            killed: killed.clone(),
            fallback,
        };
        let feed = Feed::new(&test.ctx, spec, Box::new(Shared(store.clone())));
        Rig { feed, test, key, noise, policy, pending, killed, store, _dir: dir }
    }

    fn rig(key: Option<&str>) -> Rig {
        rig_with(key, base_policy(), MemoryStore::default(), None)
    }

    fn stored(key: &str, data: u32) -> Stored<String, u32> {
        Stored { key: key.to_owned(), data, fetched_at: 5 }
    }

    impl Rig {
        fn complete(&self, index: usize, value: Option<u32>) {
            let done = self.pending.borrow_mut()[index].1.take().expect("a pending fetch");
            done(value);
        }

        fn requests(&self) -> usize {
            self.pending.borrow().len()
        }

        fn data(&self) -> Option<u32> {
            self.feed.state().data.get_untracked()
        }

        fn status(&self) -> FeedStatus {
            self.feed.state().status.get_untracked()
        }
    }

    #[test]
    fn a_fresh_store_is_published_at_once_and_nothing_is_fetched() {
        let rig = rig_with(Some("a"), base_policy(), MemoryStore::with(stored("a", 7), secs(10)), None);
        assert_eq!(rig.data(), Some(7));
        assert_eq!(rig.status(), FeedStatus::Ready);
        assert_eq!(rig.requests(), 0);
    }

    #[test]
    fn an_old_store_is_published_then_refreshed_and_a_very_old_one_is_offline() {
        let rig = rig_with(Some("a"), base_policy(), MemoryStore::with(stored("a", 7), secs(1000)), None);
        assert_eq!((rig.data(), rig.status(), rig.requests()), (Some(7), FeedStatus::Ready, 1));
        rig.complete(0, Some(8));
        assert_eq!(rig.data(), Some(8));
        assert_eq!(rig.feed.state().fetched_at.get_untracked(), Some(1_000_000));
        let old = rig_with(Some("a"), base_policy(), MemoryStore::with(stored("a", 7), secs(90_000)), None);
        assert_eq!((old.data(), old.status(), old.requests()), (Some(7), FeedStatus::Offline, 1));
    }

    #[test]
    fn a_store_for_another_key_is_not_published_but_a_missing_key_keeps_the_cache() {
        let other = rig_with(Some("b"), base_policy(), MemoryStore::with(stored("a", 7), secs(10)), None);
        assert_eq!((other.data(), other.requests()), (None, 1));
        let waiting = rig_with(None, base_policy(), MemoryStore::with(stored("a", 7), secs(10)), None);
        assert_eq!((waiting.data(), waiting.status(), waiting.requests()), (Some(7), FeedStatus::Ready, 0));
        waiting.key.set(Some("a".to_owned()));
        assert_eq!(waiting.requests(), 1);
    }

    #[test]
    fn a_successful_fetch_is_saved_and_published() {
        let rig = rig(Some("a"));
        assert_eq!(rig.status(), FeedStatus::Loading);
        assert!(rig.feed.state().loading.get_untracked());
        rig.complete(0, Some(3));
        assert_eq!((rig.data(), rig.status()), (Some(3), FeedStatus::Ready));
        assert!(!rig.feed.state().loading.get_untracked());
        assert_eq!(rig.store.saves(), 1);
        assert_eq!(rig.store.current().unwrap().data, 3);
        assert_eq!(rig.feed.state().revision.get_untracked(), 1);
    }

    #[test]
    fn unrelated_changes_read_inside_key_do_not_fetch_or_touch_the_store() {
        let rig = rig_with(Some("a"), base_policy(), MemoryStore::with(stored("a", 7), secs(10)), None);
        let loads = rig.store.loads();
        for n in 1..5 {
            rig.noise.set(n);
        }
        assert_eq!(rig.requests(), 0);
        assert_eq!(rig.store.loads(), loads);
    }

    #[test]
    fn a_key_change_waits_for_the_debounce_and_then_forces_a_fetch() {
        let policy = Policy { debounce: Duration::from_millis(300), ..base_policy() };
        let rig = rig_with(Some("a"), policy, MemoryStore::with(stored("a", 7), secs(10)), None);
        rig.key.set(Some("b".to_owned()));
        rig.key.set(Some("c".to_owned()));
        assert_eq!(rig.requests(), 0);
        assert_eq!(rig.test.scheduler.live_with_delay(300), 1);
        rig.test.scheduler.fire_with_delay(300);
        assert_eq!(rig.requests(), 1);
        assert_eq!(rig.pending.borrow()[0].0, "c");
        assert!(rig.feed.state().loading.get_untracked());
        assert_eq!(rig.data(), Some(7));
    }

    #[test]
    fn a_key_change_while_a_request_is_in_flight_kills_it_and_ignores_its_reply() {
        let rig = rig(Some("a"));
        rig.key.set(Some("b".to_owned()));
        assert_eq!(rig.killed.get(), 1);
        assert_eq!(rig.requests(), 2);
        rig.complete(0, Some(1));
        assert_eq!(rig.data(), None);
        rig.complete(1, Some(2));
        assert_eq!(rig.data(), Some(2));
        assert_eq!(rig.store.saves(), 1);
    }

    #[test]
    fn an_unforced_refresh_does_not_start_a_second_request() {
        let rig = rig(Some("a"));
        rig.feed.refresh(false);
        assert_eq!((rig.requests(), rig.killed.get()), (1, 0));
        rig.feed.refresh(true);
        assert_eq!((rig.requests(), rig.killed.get()), (2, 1));
    }

    #[test]
    fn an_interval_change_rearms_the_timer_at_once() {
        let policy = Policy { interval: Some(secs(60)), fresh_for: Duration::ZERO, ..base_policy() };
        let rig = rig_with(Some("a"), policy, MemoryStore::default(), None);
        rig.complete(0, Some(1));
        assert_eq!(rig.test.scheduler.live_with_delay(60_000), 1);
        rig.test.scheduler.fire_with_delay(60_000);
        assert_eq!(rig.requests(), 2);
        rig.policy.set(Policy { interval: Some(secs(120)), ..policy });
        assert_eq!(rig.test.scheduler.live_with_delay(60_000), 0);
        assert_eq!(rig.test.scheduler.live_with_delay(120_000), 1);
        rig.policy.set(Policy { interval: None, ..policy });
        assert_eq!(rig.test.scheduler.live(), 0);
    }

    #[test]
    fn the_interval_does_not_refetch_fresh_data() {
        let policy = Policy { interval: Some(secs(60)), ..base_policy() };
        let rig = rig_with(Some("a"), policy, MemoryStore::default(), None);
        rig.complete(0, Some(1));
        rig.test.scheduler.fire_with_delay(60_000);
        assert_eq!(rig.requests(), 1);
    }

    #[test]
    fn a_failure_keeps_fresh_data_and_retries_later() {
        let rig = rig_with(Some("a"), base_policy(), MemoryStore::with(stored("a", 7), secs(1000)), Some(0));
        rig.complete(0, None);
        assert_eq!((rig.data(), rig.status()), (Some(7), FeedStatus::Ready));
        assert_eq!(rig.test.scheduler.live_with_delay(3_600_000), 1);
        rig.test.scheduler.fire_with_delay(3_600_000);
        assert_eq!(rig.requests(), 2);
    }

    #[test]
    fn a_failure_with_stale_data_publishes_the_fallback_as_offline() {
        let rig = rig_with(Some("a"), base_policy(), MemoryStore::with(stored("a", 7), secs(90_000)), Some(0));
        rig.complete(0, None);
        assert_eq!((rig.data(), rig.status()), (Some(0), FeedStatus::Offline));
        assert_eq!(rig.store.current().unwrap().data, 0);
        let nothing = rig_with(Some("a"), base_policy(), MemoryStore::default(), None);
        nothing.complete(0, None);
        assert_eq!((nothing.data(), nothing.status()), (None, FeedStatus::Failed));
    }

    #[test]
    fn a_gated_feed_waits_for_a_subscriber() {
        let policy = Policy { gated: true, interval: Some(secs(60)), ..base_policy() };
        let rig = rig_with(Some("a"), policy, MemoryStore::default(), None);
        assert_eq!(rig.requests(), 0);
        rig.test.scheduler.fire_with_delay(60_000);
        assert_eq!(rig.requests(), 0);
        let first = rig.feed.subscribe();
        assert_eq!(rig.requests(), 1);
        let second = rig.feed.subscribe();
        assert_eq!(rig.requests(), 1);
        rig.complete(0, Some(1));
        drop(first);
        drop(second);
        rig.policy.set(Policy { fresh_for: Duration::ZERO, ..policy });
        rig.test.scheduler.fire_with_delay(60_000);
        assert_eq!(rig.requests(), 1);
        let _again = rig.feed.subscribe();
        assert_eq!(rig.requests(), 2);
    }

    #[test]
    fn cancel_aborts_the_request_and_allows_a_later_refresh() {
        let rig = rig_with(Some("a"), base_policy(), MemoryStore::default(), None);
        assert_eq!(rig.requests(), 1);
        rig.feed.cancel();
        assert_eq!(rig.killed.get(), 1);
        assert!(!rig.feed.state().loading.get());
        rig.complete(0, Some(5));
        assert_eq!(rig.data(), None);
        rig.feed.refresh(false);
        assert_eq!(rig.requests(), 2);
    }

    #[test]
    fn losing_the_key_aborts_the_request_and_keeps_the_data() {
        let rig = rig_with(Some("a"), base_policy(), MemoryStore::with(stored("a", 7), secs(1000)), None);
        rig.key.set(None);
        assert_eq!(rig.killed.get(), 1);
        assert_eq!((rig.data(), rig.status()), (Some(7), FeedStatus::Ready));
        rig.complete(0, Some(9));
        assert_eq!(rig.data(), Some(7));
    }

    #[test]
    fn without_a_key_and_without_data_the_feed_is_idle() {
        let rig = rig(Some("a"));
        rig.key.set(None);
        assert_eq!((rig.data(), rig.status()), (None, FeedStatus::Idle));
        assert!(!rig.feed.state().loading.get_untracked());
    }

    #[test]
    fn dropping_the_feed_kills_the_running_request() {
        let rig = rig(Some("a"));
        let killed = rig.killed.clone();
        drop(rig.feed);
        assert_eq!(killed.get(), 1);
    }
}
