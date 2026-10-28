use std::cell::{Cell, RefCell};
use std::rc::{Rc, Weak};
use std::time::Duration;

use acanthus_core::Signal;
use acanthus_io::{Scheduler, TimerHandle};

use super::{Cmd, Exec};

pub type DaemonSpawn = Rc<dyn Fn(Box<dyn FnMut(&str)>, Box<dyn FnOnce()>) -> Option<Box<dyn std::any::Any>>>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Restart {
    Never,
    After(Duration),
}

#[derive(Debug, Clone, Copy)]
pub struct DaemonOptions {
    pub restart: Restart,
    pub watchdog: Option<Duration>,
}

pub struct DaemonSpec {
    pub name: &'static str,
    pub cmd: Cmd,
    pub restart: Restart,
    pub watchdog: Option<Duration>,
    pub prepare: Option<Rc<dyn Fn() -> std::io::Result<()>>>,
}

#[derive(Clone)]
pub struct DaemonState {
    pub running: Signal<bool>,
    pub restarting: Signal<bool>,
}

impl DaemonState {
    pub fn new() -> DaemonState {
        DaemonState { running: Signal::new(false), restarting: Signal::new(false) }
    }
}

impl Default for DaemonState {
    fn default() -> Self {
        DaemonState::new()
    }
}

pub fn spawn_for(exec: Rc<dyn Exec>, spec: DaemonSpec) -> (DaemonSpawn, DaemonOptions) {
    let DaemonSpec { name, cmd, restart, watchdog, prepare } = spec;
    let spawn: DaemonSpawn = Rc::new(move |on_line, on_exit| {
        if let Some(prepare) = &prepare
            && let Err(error) = prepare()
        {
            tracing::warn!("{name}: cannot prepare: {error}");
            return None;
        }
        let running = exec.spawn(cmd.clone(), on_line, Box::new(move |_| on_exit()))?;
        Some(Box::new(running) as Box<dyn std::any::Any>)
    });
    (spawn, DaemonOptions { restart, watchdog })
}

struct DaemonInner {
    spawn: DaemonSpawn,
    schedule: Scheduler,
    options: DaemonOptions,
    on_line: Rc<dyn Fn(&str)>,
    on_stop: Rc<dyn Fn()>,
    state: DaemonState,
    wanted: Cell<bool>,
    watching: Cell<bool>,
    active: Cell<bool>,
    generation: Cell<u64>,
    process: RefCell<Option<Box<dyn std::any::Any>>>,
    restart_timer: RefCell<Option<TimerHandle>>,
    watchdog_timer: RefCell<Option<TimerHandle>>,
    watchdog_armed: Cell<bool>,
}

pub struct Daemon {
    inner: Rc<DaemonInner>,
    pub state: DaemonState,
}

impl DaemonInner {
    fn arm(self: &Rc<Self>, slot: &RefCell<Option<TimerHandle>>, delay: Duration, action: fn(&Rc<DaemonInner>)) {
        let weak: Weak<DaemonInner> = Rc::downgrade(self);
        let handle = (self.schedule)(
            delay,
            Box::new(move || {
                if let Some(inner) = weak.upgrade() {
                    action(&inner);
                }
                None
            }),
        );
        *slot.borrow_mut() = handle;
    }

    fn arm_watchdog(self: &Rc<Self>) {
        if let Some(limit) = self.options.watchdog {
            self.watchdog_armed.set(true);
            self.arm(&self.watchdog_timer, limit, |inner| {
                inner.watchdog_armed.set(false);
                inner.restart();
            });
        }
    }

    fn disarm_watchdog(&self) {
        self.watchdog_timer.borrow_mut().take();
        self.watchdog_armed.set(false);
    }

    fn sync(self: &Rc<Self>) {
        let active = self.wanted.get() && !self.state.restarting.get_untracked();
        let was = self.active.replace(active);
        if was && !active {
            (self.on_stop)();
            self.disarm_watchdog();
        }
        if active && self.process.borrow().is_none() {
            self.start_process();
        }
        if !active {
            let stopped = self.process.borrow_mut().take();
            drop(stopped);
        }
        let running = self.process.borrow().is_some();
        self.state.running.set(running);
        let watch = running && self.watching.get() && self.options.watchdog.is_some();
        if watch && !self.watchdog_armed.get() {
            self.arm_watchdog();
        } else if !watch {
            self.disarm_watchdog();
        }
    }

    fn start_process(self: &Rc<Self>) {
        let generation = self.generation.get() + 1;
        self.generation.set(generation);
        let line_target = Rc::downgrade(self);
        let exit_target = Rc::downgrade(self);
        let process = (self.spawn)(
            Box::new(move |line| {
                if let Some(inner) = line_target.upgrade().filter(|inner| inner.generation.get() == generation) {
                    inner.line(line);
                }
            }),
            Box::new(move || {
                if let Some(inner) = exit_target.upgrade().filter(|inner| inner.generation.get() == generation) {
                    inner.exited();
                }
            }),
        );
        match process {
            Some(handle) => *self.process.borrow_mut() = Some(handle),
            None => {
                self.generation.set(generation + 1);
                self.restart();
            }
        }
    }

    fn line(self: &Rc<Self>, line: &str) {
        if !self.active.get() {
            return;
        }
        if self.watchdog_armed.get() {
            self.arm_watchdog();
        }
        (self.on_line)(line);
    }

    fn exited(self: &Rc<Self>) {
        self.process.borrow_mut().take();
        self.state.running.set(false);
        self.disarm_watchdog();
        self.restart();
    }

    fn restart(self: &Rc<Self>) {
        if !self.active.get() {
            return;
        }
        let Restart::After(delay) = self.options.restart else { return };
        self.state.restarting.set(true);
        self.sync();
        self.arm(&self.restart_timer, delay, |inner| {
            inner.state.restarting.set(false);
            inner.sync();
        });
    }
}

impl Daemon {
    pub fn new(
        exec: Rc<dyn Exec>,
        schedule: Scheduler,
        spec: DaemonSpec,
        state: DaemonState,
        on_line: Rc<dyn Fn(&str)>,
        on_stop: Rc<dyn Fn()>,
    ) -> Daemon {
        let (spawn, options) = spawn_for(exec, spec);
        Daemon::with_spawn(spawn, schedule, options, state, on_line, on_stop)
    }

    pub fn with_spawn(
        spawn: DaemonSpawn,
        schedule: Scheduler,
        options: DaemonOptions,
        state: DaemonState,
        on_line: Rc<dyn Fn(&str)>,
        on_stop: Rc<dyn Fn()>,
    ) -> Daemon {
        let inner = Rc::new(DaemonInner {
            spawn,
            schedule,
            options,
            on_line,
            on_stop,
            state: state.clone(),
            wanted: Cell::new(false),
            watching: Cell::new(false),
            active: Cell::new(false),
            generation: Cell::new(0),
            process: RefCell::new(None),
            restart_timer: RefCell::new(None),
            watchdog_timer: RefCell::new(None),
            watchdog_armed: Cell::new(false),
        });
        Daemon { inner, state }
    }

    pub fn set_enabled(&self, enabled: bool) {
        self.inner.wanted.set(enabled);
        self.inner.sync();
    }

    pub fn set_watching(&self, watching: bool) {
        self.inner.watching.set(watching);
        self.inner.sync();
    }
}

impl Drop for Daemon {
    fn drop(&mut self) {
        self.inner.generation.set(self.inner.generation.get() + 1);
        self.inner.process.borrow_mut().take();
        self.inner.restart_timer.borrow_mut().take();
        self.inner.watchdog_timer.borrow_mut().take();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    type LineFeed = Rc<RefCell<Option<Box<dyn FnMut(&str)>>>>;
    type ExitFeed = Rc<RefCell<Option<Box<dyn FnOnce()>>>>;
    type StreamTimers = Rc<RefCell<Vec<(u64, Rc<RefCell<Option<Box<dyn FnMut() -> Option<Duration>>>>>)>>>;

    struct DaemonRig {
        lines: Rc<RefCell<Vec<String>>>,
        stops: Rc<Cell<u32>>,
        spawns: Rc<Cell<u32>>,
        alive: Rc<Cell<u32>>,
        feed: LineFeed,
        exit: ExitFeed,
        timers: StreamTimers,
        stream: Daemon,
    }

    struct Alive(Rc<Cell<u32>>);

    impl Drop for Alive {
        fn drop(&mut self) {
            self.0.set(self.0.get() - 1);
        }
    }

    fn stream_rig(watchdog: Option<Duration>) -> DaemonRig {
        let lines = Rc::new(RefCell::new(Vec::new()));
        let stops = Rc::new(Cell::new(0));
        let spawns = Rc::new(Cell::new(0));
        let alive = Rc::new(Cell::new(0));
        let feed: LineFeed = Rc::new(RefCell::new(None));
        let exit: ExitFeed = Rc::new(RefCell::new(None));
        let (s, a, f, e) = (spawns.clone(), alive.clone(), feed.clone(), exit.clone());
        let spawn: DaemonSpawn = Rc::new(move |on_line, on_exit| {
            s.set(s.get() + 1);
            a.set(a.get() + 1);
            *f.borrow_mut() = Some(on_line);
            *e.borrow_mut() = Some(on_exit);
            Some(Box::new(Alive(a.clone())) as Box<dyn std::any::Any>)
        });
        let manual = crate::kit::testing::ManualScheduler::new();
        let timers = manual.timers();
        let schedule = manual.scheduler();
        let sink = lines.clone();
        let stop_counter = stops.clone();
        let stream = Daemon::with_spawn(
            spawn,
            schedule,
            DaemonOptions { restart: Restart::After(Duration::from_millis(500)), watchdog },
            DaemonState::new(),
            Rc::new(move |line| sink.borrow_mut().push(line.to_owned())),
            Rc::new(move || stop_counter.set(stop_counter.get() + 1)),
        );
        DaemonRig { lines, stops, spawns, alive, feed, exit, timers, stream }
    }

    impl DaemonRig {
        fn send(&self, text: &str) {
            let mut taken = self.feed.borrow_mut().take();
            if let Some(f) = taken.as_mut() {
                f(text);
            }
            *self.feed.borrow_mut() = taken;
        }

        fn die(&self) {
            let taken = self.exit.borrow_mut().take();
            if let Some(f) = taken {
                f();
            }
        }

        fn fire(&self, delay: u64) {
            let slots: Vec<_> =
                self.timers.borrow().iter().filter(|(d, _)| *d == delay).map(|(_, s)| s.clone()).collect();
            for slot in slots {
                let taken = slot.borrow_mut().take();
                if let Some(mut f) = taken {
                    f();
                }
            }
        }

        fn pending(&self, delay: u64) -> usize {
            self.timers.borrow().iter().filter(|(d, slot)| *d == delay && slot.borrow().is_some()).count()
        }
    }

    #[test]
    fn stream_runs_only_while_enabled_and_forwards_lines() {
        let rig = stream_rig(None);
        assert_eq!(rig.spawns.get(), 0);
        rig.stream.set_enabled(true);
        assert_eq!(rig.spawns.get(), 1);
        assert!(rig.stream.state.running.get());
        rig.send("a");
        rig.send("b");
        assert_eq!(*rig.lines.borrow(), ["a", "b"]);
        rig.stream.set_enabled(false);
        assert_eq!(rig.alive.get(), 0);
        assert_eq!(rig.stops.get(), 1);
        assert!(!rig.stream.state.running.get());
        rig.stream.set_enabled(false);
        assert_eq!(rig.stops.get(), 1);
    }

    #[test]
    fn stream_restarts_after_the_delay_when_the_process_dies() {
        let rig = stream_rig(None);
        rig.stream.set_enabled(true);
        for round in 1..=3 {
            rig.die();
            assert!(rig.stream.state.restarting.get());
            assert_eq!(rig.alive.get(), 0);
            assert_eq!(rig.pending(500), 1);
            rig.fire(500);
            assert!(!rig.stream.state.restarting.get());
            assert_eq!(rig.spawns.get(), round + 1);
        }
        assert_eq!(rig.stops.get(), 3);
    }

    #[test]
    fn stream_watchdog_restarts_a_silent_process_only_while_watching() {
        let rig = stream_rig(Some(Duration::from_millis(2500)));
        rig.stream.set_enabled(true);
        assert_eq!(rig.pending(2500), 0);
        rig.stream.set_watching(true);
        assert_eq!(rig.pending(2500), 1);
        rig.send("x");
        assert_eq!(rig.pending(2500), 1);
        rig.fire(2500);
        assert!(rig.stream.state.restarting.get());
        rig.fire(500);
        assert_eq!(rig.spawns.get(), 2);
        assert_eq!(rig.pending(2500), 1);
        rig.stream.set_watching(false);
        assert_eq!(rig.pending(2500), 0);
    }

    #[test]
    fn stream_ignores_callbacks_of_replaced_processes() {
        let rig = stream_rig(None);
        rig.stream.set_enabled(true);
        let old_exit = rig.exit.borrow_mut().take().unwrap();
        rig.stream.set_enabled(false);
        rig.stream.set_enabled(true);
        assert_eq!(rig.spawns.get(), 2);
        old_exit();
        assert!(!rig.stream.state.restarting.get());
        assert_eq!(rig.alive.get(), 1);
    }
}
