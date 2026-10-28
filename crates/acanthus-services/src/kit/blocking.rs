use std::any::Any;
use std::cell::Cell;
use std::rc::Rc;

use acanthus_io::WeakHandle;
use calloop::LoopHandle;
use calloop::channel::{Event, channel};

type Output = Box<dyn Any + Send>;
type Job = Box<dyn FnOnce() -> Output + Send>;
type Done = Box<dyn FnOnce(Output)>;
type Launch = Rc<dyn Fn(Job, Done)>;

/// Runs blocking work on a helper thread and delivers the result back on the main loop.
#[derive(Clone)]
pub struct Blocking {
    launch: Launch,
}

impl Blocking {
    /// Creates a runner delivering results through the event loop behind `handle`.
    pub fn new<D: 'static>(handle: &LoopHandle<'static, D>) -> Blocking {
        let weak = WeakHandle::new(handle);
        Blocking {
            launch: Rc::new(move |job, done| {
                let Some(handle) = weak.upgrade() else { return };
                let (sender, receiver) = channel::<Output>();
                let mut done = Some(done);
                let token = Rc::new(Cell::new(None));
                let slot = token.clone();
                let remover = WeakHandle::new(&handle);
                let inserted = handle.insert_source(receiver, move |event, _, _| {
                    if let Event::Msg(output) = event {
                        if let Some(done) = done.take() {
                            done(output);
                        }
                        if let Some(token) = slot.take() {
                            remover.remove(token);
                        }
                    }
                });
                let Ok(registration) = inserted else {
                    tracing::warn!("cannot register a blocking job");
                    return;
                };
                token.set(Some(registration));
                std::thread::spawn(move || {
                    if sender.send(job()).is_err() {
                        tracing::debug!("a blocking job finished after its loop was gone");
                    }
                });
            }),
        }
    }

    /// A runner that executes the job on the calling thread and delivers immediately; for tests.
    pub fn inline() -> Blocking {
        Blocking { launch: Rc::new(|job, done| done(job())) }
    }

    /// Runs `job` off the main loop and then `done` with its result on the main loop. The result is dropped when the loop is gone.
    pub fn run<T: Send + 'static>(&self, job: impl FnOnce() -> T + Send + 'static, done: impl FnOnce(T) + 'static) {
        (self.launch)(
            Box::new(move || Box::new(job()) as Output),
            Box::new(move |output| match output.downcast::<T>() {
                Ok(value) => done(*value),
                Err(_) => tracing::error!("a blocking job returned an unexpected type"),
            }),
        );
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::time::{Duration, Instant};

    use calloop::EventLoop;

    use super::*;

    #[test]
    fn inline_runs_the_job_and_the_callback_at_once() {
        let seen = Rc::new(RefCell::new(None));
        let sink = seen.clone();
        Blocking::inline().run(|| 40 + 2, move |value| *sink.borrow_mut() = Some(value));
        assert_eq!(*seen.borrow(), Some(42));
    }

    #[test]
    fn the_result_returns_on_the_loop_thread() {
        let mut event_loop: EventLoop<'static, ()> = EventLoop::try_new().unwrap();
        let blocking = Blocking::new(&event_loop.handle());
        let main = std::thread::current().id();
        let seen = Rc::new(RefCell::new(None));
        let sink = seen.clone();
        blocking.run(
            move || {
                std::thread::sleep(Duration::from_millis(20));
                (std::thread::current().id() != main, "done".to_owned())
            },
            move |value| *sink.borrow_mut() = Some((value, std::thread::current().id() == main)),
        );
        let deadline = Instant::now() + Duration::from_secs(5);
        while seen.borrow().is_none() && Instant::now() < deadline {
            event_loop.dispatch(Duration::from_millis(10), &mut ()).unwrap();
        }
        let ((off_thread, text), back_on_loop) = seen.borrow().clone().expect("finished");
        assert!(off_thread && back_on_loop);
        assert_eq!(text, "done");
    }

    #[test]
    fn many_jobs_complete_independently() {
        let mut event_loop: EventLoop<'static, ()> = EventLoop::try_new().unwrap();
        let blocking = Blocking::new(&event_loop.handle());
        let total = Rc::new(std::cell::Cell::new(0));
        for n in 1..=5 {
            let sink = total.clone();
            blocking.run(move || n, move |value| sink.set(sink.get() + value));
        }
        let deadline = Instant::now() + Duration::from_secs(5);
        while total.get() < 15 && Instant::now() < deadline {
            event_loop.dispatch(Duration::from_millis(10), &mut ()).unwrap();
        }
        assert_eq!(total.get(), 15);
    }
}
