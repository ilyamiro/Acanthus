use std::future::Future;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};

use acanthus_io::{IoError, WeakHandle};
use async_executor::LocalExecutor;
use calloop::channel::{Channel, Event, Sender, channel};
use calloop::{LoopHandle, RegistrationToken};

/// Sends events from a worker thread to the main loop. Cloning shares the channel.
pub struct EventSink<E>(Sender<E>);

impl<E> EventSink<E> {
    /// Sends an event; false when the receiving side is gone.
    pub fn send(&self, event: E) -> bool {
        self.0.send(event).is_ok()
    }
}

impl<E> Clone for EventSink<E> {
    fn clone(&self) -> Self {
        EventSink(self.0.clone())
    }
}

/// Sends commands to a worker task; dropping it closes the channel and unregisters the event source.
pub struct ServiceHandle<C> {
    commands: async_channel::Sender<C>,
    stop: async_channel::Sender<()>,
    remove: Option<Box<dyn FnOnce()>>,
}

impl<C> ServiceHandle<C> {
    /// Sends a command; false when the worker is gone or its queue is closed.
    pub fn send(&self, command: C) -> bool {
        self.commands.try_send(command).is_ok()
    }
}

impl<C> Drop for ServiceHandle<C> {
    fn drop(&mut self) {
        self.commands.close();
        self.stop.close();
        if let Some(remove) = self.remove.take() {
            remove();
        }
    }
}

/// A flag a worker thread polls to learn that it should stop.
#[derive(Clone, Default)]
pub struct StopFlag(Arc<AtomicBool>);

impl StopFlag {
    /// True once stop was requested.
    pub fn is_set(&self) -> bool {
        self.0.load(Ordering::Relaxed)
    }

    /// Sleeps up to `duration`, waking early when the flag is set; returns true when stopped.
    pub fn sleep(&self, duration: Duration) -> bool {
        let end = Instant::now() + duration;
        while Instant::now() < end {
            if self.is_set() {
                return true;
            }
            std::thread::sleep(Duration::from_millis(20).min(end.saturating_duration_since(Instant::now())));
        }
        self.is_set()
    }
}

/// Handle of a thread started by [`spawn_thread`]; dropping it sets the stop flag and unregisters the event source.
pub struct ThreadHandle {
    stop: StopFlag,
    remove: Option<Box<dyn FnOnce()>>,
}

impl ThreadHandle {
    /// The flag the thread was given.
    pub fn stop_flag(&self) -> StopFlag {
        self.stop.clone()
    }
}

impl Drop for ThreadHandle {
    fn drop(&mut self) {
        self.stop.0.store(true, Ordering::Relaxed);
        if let Some(remove) = self.remove.take() {
            remove();
        }
    }
}

/// Starts a thread named `name` running `run` and delivers the events it sends to `on_event` on the main loop.
pub fn spawn_thread<D, E>(
    handle: &LoopHandle<'static, D>,
    name: &str,
    mut on_event: impl FnMut(E) + 'static,
    run: impl FnOnce(EventSink<E>, StopFlag) + Send + 'static,
) -> Result<ThreadHandle, IoError>
where
    D: 'static,
    E: Send + 'static,
{
    let (sender, receiver): (Sender<E>, Channel<E>) = channel();
    let token: RegistrationToken = handle.insert_source(receiver, move |event, _, _| {
        if let Event::Msg(event) = event {
            on_event(event);
        }
    })?;
    let stop = StopFlag::default();
    let worker_stop = stop.clone();
    std::thread::Builder::new().name(name.to_owned()).spawn(move || run(EventSink(sender), worker_stop))?;
    let weak = WeakHandle::new(handle);
    Ok(ThreadHandle { stop, remove: Some(Box::new(move || weak.remove(token))) })
}

/// Starts a thread named `name` that runs the async `run` with an event sink and a command receiver, delivering events to `on_event` on the main loop.
pub fn spawn_worker<D, E, C, F, Fut>(
    handle: &LoopHandle<'static, D>,
    name: &str,
    on_event: impl FnMut(E) + 'static,
    run: F,
) -> Result<ServiceHandle<C>, IoError>
where
    D: 'static,
    E: Send + 'static,
    C: Send + 'static,
    F: FnOnce(EventSink<E>, async_channel::Receiver<C>) -> Fut + Send + 'static,
    Fut: Future<Output = ()> + 'static,
{
    let name = name.to_owned();
    spawn_with(handle, on_event, move |events, commands, stop| {
        std::thread::Builder::new().name(name).spawn(move || {
            async_io::block_on(until_stopped(run(events, commands), &stop));
        })?;
        Ok(())
    })
}

fn spawn_with<D, E, C>(
    handle: &LoopHandle<'static, D>,
    mut on_event: impl FnMut(E) + 'static,
    launch: impl FnOnce(EventSink<E>, async_channel::Receiver<C>, async_channel::Receiver<()>) -> Result<(), IoError>,
) -> Result<ServiceHandle<C>, IoError>
where
    D: 'static,
    E: Send + 'static,
    C: Send + 'static,
{
    let (sender, receiver): (Sender<E>, Channel<E>) = channel();
    let token: RegistrationToken = handle.insert_source(receiver, move |event, _, _| {
        if let Event::Msg(event) = event {
            on_event(event);
        }
    })?;
    let (command_tx, command_rx) = async_channel::unbounded();
    let (stop_tx, stop_rx) = async_channel::bounded(1);
    let weak = WeakHandle::new(handle);
    if let Err(error) = launch(EventSink(sender), command_rx, stop_rx) {
        weak.remove(token);
        return Err(error);
    }
    Ok(ServiceHandle { commands: command_tx, stop: stop_tx, remove: Some(Box::new(move || weak.remove(token))) })
}

async fn until_stopped(work: impl Future<Output = ()>, stop: &async_channel::Receiver<()>) -> bool {
    futures_lite::future::or(
        async {
            work.await;
            false
        },
        async {
            let _ = stop.recv().await;
            true
        },
    )
    .await
}

type Job = Box<dyn FnOnce(&LocalExecutor<'static>) + Send>;

fn executor() -> Result<&'static async_channel::Sender<Job>, IoError> {
    static EXECUTOR: OnceLock<async_channel::Sender<Job>> = OnceLock::new();
    if let Some(sender) = EXECUTOR.get() {
        return Ok(sender);
    }
    let (sender, jobs) = async_channel::unbounded::<Job>();
    std::thread::Builder::new().name("dbus-executor".to_owned()).spawn(move || {
        let local = LocalExecutor::new();
        async_io::block_on(local.run(async {
            while let Ok(job) = jobs.recv().await {
                job(&local);
            }
        }));
    })?;
    Ok(EXECUTOR.get_or_init(|| sender))
}

/// Restart delays for supervised workers: the first delay, the longest delay and how long a run must last to count as healthy.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Backoff {
    /// Delay after the first failure.
    pub first: Duration,
    /// Longest delay; the delay doubles up to this.
    pub max: Duration,
    /// A run lasting at least this long resets the delay.
    pub healthy: Duration,
}

impl Backoff {
    /// One second first, thirty seconds at most, healthy after a minute.
    pub const DEFAULT: Backoff =
        Backoff { first: Duration::from_secs(1), max: Duration::from_secs(30), healthy: Duration::from_secs(60) };

    /// The delay before the next restart, given the previous delay and how long the last run lasted.
    pub fn next(&self, previous: Option<Duration>, ran_for: Duration) -> Duration {
        match previous {
            Some(delay) if ran_for < self.healthy => (delay * 2).min(self.max),
            _ => self.first,
        }
    }
}

/// Like [`spawn_worker`] but the worker is restarted with the default backoff when it returns.
pub fn spawn_supervised<D, E, C, F, Fut>(
    handle: &LoopHandle<'static, D>,
    name: &str,
    on_event: impl FnMut(E) + 'static,
    run: F,
) -> Result<ServiceHandle<C>, IoError>
where
    D: 'static,
    E: Send + 'static,
    C: Send + 'static,
    F: Fn(EventSink<E>, async_channel::Receiver<C>) -> Fut + Send + 'static,
    Fut: Future<Output = ()> + 'static,
{
    spawn_supervised_with(handle, name, Backoff::DEFAULT, on_event, run)
}

/// Like [`spawn_supervised`] with an explicit backoff.
pub fn spawn_supervised_with<D, E, C, F, Fut>(
    handle: &LoopHandle<'static, D>,
    name: &str,
    backoff: Backoff,
    on_event: impl FnMut(E) + 'static,
    run: F,
) -> Result<ServiceHandle<C>, IoError>
where
    D: 'static,
    E: Send + 'static,
    C: Send + 'static,
    F: Fn(EventSink<E>, async_channel::Receiver<C>) -> Fut + Send + 'static,
    Fut: Future<Output = ()> + 'static,
{
    let label = name.to_owned();
    spawn_with(handle, on_event, move |events, commands, stop| {
        let job: Job = Box::new(move |local| {
            local
                .spawn(async move {
                    let mut delay: Option<Duration> = None;
                    loop {
                        let started = Instant::now();
                        if until_stopped(run(events.clone(), commands.clone()), &stop).await || commands.is_closed() {
                            return;
                        }
                        let wait = backoff.next(delay, started.elapsed());
                        tracing::warn!("{label} stopped, restarting in {wait:?}");
                        crate::platform::dbus::forget_all_shared();
                        delay = Some(wait);
                        let slept = until_stopped(
                            async {
                                async_io::Timer::after(wait).await;
                            },
                            &stop,
                        )
                        .await;
                        if slept || commands.is_closed() {
                            return;
                        }
                    }
                })
                .detach();
        });
        executor()?.try_send(job).map_err(|_| IoError::Ipc("dbus executor stopped".into()))
    })
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::rc::Rc;
    use std::time::{Duration, Instant};

    use calloop::EventLoop;

    use super::*;

    #[test]
    fn events_and_commands_cross_the_thread_boundary() {
        let mut event_loop: EventLoop<'static, ()> = EventLoop::try_new().unwrap();
        let seen = Rc::new(RefCell::new(Vec::new()));
        let sink = seen.clone();
        let service = spawn_worker(
            &event_loop.handle(),
            "echo",
            move |event: String| sink.borrow_mut().push(event),
            |events, commands: async_channel::Receiver<String>| async move {
                events.send("ready".into());
                while let Ok(command) = commands.recv().await {
                    events.send(command.to_uppercase());
                }
            },
        )
        .unwrap();
        assert!(service.send("hello".into()));
        let deadline = Instant::now() + Duration::from_secs(5);
        while seen.borrow().len() < 2 && Instant::now() < deadline {
            event_loop.dispatch(Duration::from_millis(10), &mut ()).unwrap();
        }
        assert_eq!(*seen.borrow(), ["ready", "HELLO"]);
    }

    #[test]
    fn dropping_the_handle_stops_the_worker() {
        let mut event_loop: EventLoop<'static, ()> = EventLoop::try_new().unwrap();
        let service = spawn_worker(
            &event_loop.handle(),
            "stopper",
            |_: ()| {},
            |_events, commands: async_channel::Receiver<()>| async move { while commands.recv().await.is_ok() {} },
        )
        .unwrap();
        drop(service);
        event_loop.dispatch(Duration::from_millis(10), &mut ()).unwrap();
    }

    #[test]
    fn supervised_workers_share_one_executor_thread() {
        let mut event_loop: EventLoop<'static, ()> = EventLoop::try_new().unwrap();
        let seen = Rc::new(RefCell::new(Vec::new()));
        let mut services = Vec::new();
        for _ in 0..2 {
            let sink = seen.clone();
            services.push(
                spawn_supervised(
                    &event_loop.handle(),
                    "shared",
                    move |name: String| sink.borrow_mut().push(name),
                    |events, commands: async_channel::Receiver<()>| async move {
                        events.send(std::thread::current().name().unwrap_or_default().to_owned());
                        while commands.recv().await.is_ok() {}
                    },
                )
                .unwrap(),
            );
        }
        let deadline = Instant::now() + Duration::from_secs(5);
        while seen.borrow().len() < 2 && Instant::now() < deadline {
            event_loop.dispatch(Duration::from_millis(10), &mut ()).unwrap();
        }
        assert_eq!(*seen.borrow(), ["dbus-executor", "dbus-executor"]);
    }

    #[test]
    fn backoff_doubles_until_the_cap_and_resets_after_a_healthy_run() {
        let b =
            Backoff { first: Duration::from_secs(1), max: Duration::from_secs(8), healthy: Duration::from_secs(60) };
        assert_eq!(b.next(None, Duration::ZERO), Duration::from_secs(1));
        assert_eq!(b.next(Some(Duration::from_secs(1)), Duration::from_secs(2)), Duration::from_secs(2));
        assert_eq!(b.next(Some(Duration::from_secs(4)), Duration::from_secs(2)), Duration::from_secs(8));
        assert_eq!(b.next(Some(Duration::from_secs(8)), Duration::from_secs(2)), Duration::from_secs(8));
        assert_eq!(b.next(Some(Duration::from_secs(8)), Duration::from_secs(61)), Duration::from_secs(1));
    }

    #[test]
    fn a_worker_that_ends_is_restarted_until_the_handle_drops() {
        let mut event_loop: EventLoop<'static, ()> = EventLoop::try_new().unwrap();
        let starts = Rc::new(RefCell::new(0u32));
        let seen = starts.clone();
        let backoff = Backoff {
            first: Duration::from_millis(5),
            max: Duration::from_millis(10),
            healthy: Duration::from_secs(60),
        };
        let service = spawn_supervised_with(
            &event_loop.handle(),
            "flaky",
            backoff,
            move |_: u32| *seen.borrow_mut() += 1,
            |events, _commands: async_channel::Receiver<()>| async move {
                events.send(1);
            },
        )
        .unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        while *starts.borrow() < 3 && Instant::now() < deadline {
            event_loop.dispatch(Duration::from_millis(10), &mut ()).unwrap();
        }
        assert!(*starts.borrow() >= 3);
        drop(service);
        event_loop.dispatch(Duration::from_millis(30), &mut ()).unwrap();
        let after = *starts.borrow();
        event_loop.dispatch(Duration::from_millis(60), &mut ()).unwrap();
        assert_eq!(*starts.borrow(), after);
    }

    struct Finished(std::sync::mpsc::Sender<&'static str>);

    impl Drop for Finished {
        fn drop(&mut self) {
            let _ = self.0.send("finished");
        }
    }

    #[test]
    fn a_worker_deaf_to_commands_stops_with_its_handle() {
        let event_loop: EventLoop<'static, ()> = EventLoop::try_new().unwrap();
        for supervised in [false, true] {
            let (done_tx, done_rx) = std::sync::mpsc::channel();
            let run = move |_events: EventSink<()>, _commands: async_channel::Receiver<()>| {
                let guard = Finished(done_tx.clone());
                async move {
                    let _guard = guard;
                    futures_lite::future::pending::<()>().await;
                }
            };
            let service = if supervised {
                spawn_supervised(&event_loop.handle(), "deaf", |_: ()| {}, run).unwrap()
            } else {
                spawn_worker(&event_loop.handle(), "deaf", |_: ()| {}, run).unwrap()
            };
            assert!(done_rx.recv_timeout(Duration::from_millis(100)).is_err(), "supervised {supervised}");
            drop(service);
            assert_eq!(done_rx.recv_timeout(Duration::from_secs(5)), Ok("finished"), "supervised {supervised}");
        }
    }

    #[test]
    fn a_supervised_worker_waiting_to_restart_stops_with_its_handle() {
        let event_loop: EventLoop<'static, ()> = EventLoop::try_new().unwrap();
        let (runs_tx, runs_rx) = std::sync::mpsc::channel();
        let (done_tx, done_rx) = std::sync::mpsc::channel();
        let supervisor_alive = Finished(done_tx);
        let backoff = Backoff { first: Duration::from_secs(60), max: Duration::from_secs(60), healthy: Duration::ZERO };
        let service = spawn_supervised_with(
            &event_loop.handle(),
            "flaky",
            backoff,
            |_: ()| {},
            move |_events, _commands: async_channel::Receiver<()>| {
                let _ = &supervisor_alive;
                let runs = runs_tx.clone();
                async move {
                    let _ = runs.send(());
                }
            },
        )
        .unwrap();
        runs_rx.recv_timeout(Duration::from_secs(5)).unwrap();
        assert!(done_rx.recv_timeout(Duration::from_millis(100)).is_err());
        drop(service);
        assert_eq!(done_rx.recv_timeout(Duration::from_secs(5)), Ok("finished"));
        assert!(runs_rx.try_recv().is_err());
    }
}
