use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;
use std::time::Duration;

use acanthus_io::{ExitInfo, Process, ProcessHandle, TimerHandle, WeakHandle};
use calloop::LoopHandle;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Cmd {
    pub argv: Vec<String>,
    pub timeout: Option<Duration>,
    pub env: Vec<(String, String)>,
    pub cwd: Option<PathBuf>,
}

impl Cmd {
    pub fn new<I, S>(argv: I) -> Cmd
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        Cmd { argv: argv.into_iter().map(Into::into).collect(), ..Cmd::default() }
    }

    pub fn timeout(mut self, timeout: Duration) -> Cmd {
        self.timeout = Some(timeout);
        self
    }

    pub fn env(mut self, key: impl Into<String>, value: impl Into<String>) -> Cmd {
        self.env.push((key.into(), value.into()));
        self
    }

    pub fn cwd(mut self, dir: impl Into<PathBuf>) -> Cmd {
        self.cwd = Some(dir.into());
        self
    }
}

pub trait Running {
    fn pid(&self) -> Option<u32>;
    fn kill(&self);
}

pub type Done = Box<dyn FnOnce(ExitInfo)>;

pub trait Exec {
    fn run(&self, cmd: Cmd, done: Done) -> Option<Box<dyn Running>>;
    fn spawn(&self, cmd: Cmd, on_line: Box<dyn FnMut(&str)>, on_exit: Done) -> Option<Box<dyn Running>>;
    fn detached(&self, cmd: Cmd);
    fn has_program(&self, name: &str) -> bool;
}

pub struct ProcessExec<D: 'static> {
    handle: WeakHandle<D>,
}

impl<D: 'static> ProcessExec<D> {
    pub fn new(handle: &LoopHandle<'static, D>) -> ProcessExec<D> {
        ProcessExec { handle: WeakHandle::new(handle) }
    }

    pub fn shared(handle: &LoopHandle<'static, D>) -> Rc<dyn Exec> {
        Rc::new(ProcessExec::new(handle))
    }

    fn process(cmd: &Cmd) -> Option<Process> {
        let mut parts = cmd.argv.iter();
        let mut process = Process::new(parts.next()?).args(parts);
        for (key, value) in &cmd.env {
            process = process.env(key, value);
        }
        if let Some(dir) = &cmd.cwd {
            process = process.cwd(dir);
        }
        Some(process)
    }
}

fn failed() -> ExitInfo {
    ExitInfo { code: None, ..Default::default() }
}

struct Child {
    handle: Rc<ProcessHandle>,
    timer: Rc<RefCell<Option<TimerHandle>>>,
}

impl Running for Child {
    fn pid(&self) -> Option<u32> {
        u32::try_from(self.handle.pid()).ok()
    }

    fn kill(&self) {
        self.handle.kill();
    }
}

impl Drop for Child {
    fn drop(&mut self) {
        self.timer.borrow_mut().take();
    }
}

impl<D: 'static> Exec for ProcessExec<D> {
    fn run(&self, cmd: Cmd, done: Done) -> Option<Box<dyn Running>> {
        let (Some(handle), Some(process)) = (self.handle.upgrade(), ProcessExec::<D>::process(&cmd)) else {
            tracing::warn!("cannot run {:?}", cmd.argv);
            done(failed());
            return None;
        };
        let pending = Rc::new(RefCell::new(Some(done)));
        let timer: Rc<RefCell<Option<TimerHandle>>> = Rc::new(RefCell::new(None));
        let (on_exit, expiry) = (pending.clone(), timer.clone());
        let spawned = process
            .collect_output()
            .kill_on_drop(false)
            .on_exit(move |info| {
                expiry.borrow_mut().take();
                if let Some(done) = on_exit.borrow_mut().take() {
                    done(info);
                }
            })
            .spawn(&handle);
        match spawned {
            Ok(process) => {
                let process = Rc::new(process);
                if let Some(limit) = cmd.timeout {
                    let target = process.clone();
                    *timer.borrow_mut() = acanthus_io::timer::after(&handle, limit, move || target.kill()).ok();
                }
                Some(Box::new(Child { handle: process, timer }))
            }
            Err(error) => {
                tracing::warn!("spawn of {:?} failed: {error}", cmd.argv);
                if let Some(done) = pending.borrow_mut().take() {
                    done(failed());
                }
                None
            }
        }
    }

    fn spawn(&self, cmd: Cmd, on_line: Box<dyn FnMut(&str)>, on_exit: Done) -> Option<Box<dyn Running>> {
        let (Some(handle), Some(process)) = (self.handle.upgrade(), ProcessExec::<D>::process(&cmd)) else {
            tracing::warn!("cannot spawn {:?}", cmd.argv);
            return None;
        };
        let mut on_line = on_line;
        match process.on_stdout_line(move |line| on_line(line)).on_exit(on_exit).spawn(&handle) {
            Ok(process) => Some(Box::new(Child { handle: Rc::new(process), timer: Rc::new(RefCell::new(None)) })),
            Err(error) => {
                tracing::warn!("spawn of {:?} failed: {error}", cmd.argv);
                None
            }
        }
    }

    fn detached(&self, cmd: Cmd) {
        if let Err(error) = acanthus_io::exec_detached(cmd.argv.clone()) {
            tracing::warn!("detached start of {:?} failed: {error}", cmd.argv);
        }
    }

    fn has_program(&self, name: &str) -> bool {
        program_in_path(name)
    }
}

pub fn run_blocking(cmd: &Cmd) -> Option<ExitInfo> {
    acanthus_io::run_to_completion(&cmd.argv).ok()
}

pub fn program_in_path(name: &str) -> bool {
    if name.contains('/') {
        return std::path::Path::new(name).is_file();
    }
    std::env::var_os("PATH").is_some_and(|paths| std::env::split_paths(&paths).any(|dir| dir.join(name).is_file()))
}

#[cfg(test)]
mod tests {
    use std::cell::{Cell, RefCell};
    use std::time::Instant;

    use calloop::EventLoop;

    use super::*;

    type TestLoop = EventLoop<'static, ()>;

    fn pump(event_loop: &mut TestLoop, done: impl Fn() -> bool) {
        let deadline = Instant::now() + Duration::from_secs(5);
        while !done() && Instant::now() < deadline {
            event_loop.dispatch(Duration::from_millis(10), &mut ()).unwrap();
        }
    }

    fn sh(script: &str) -> Cmd {
        Cmd::new(["sh", "-c", script])
    }

    #[test]
    fn run_collects_stdout_and_the_exit_code() {
        let mut event_loop: TestLoop = EventLoop::try_new().unwrap();
        let exec = ProcessExec::shared(&event_loop.handle());
        let result = Rc::new(RefCell::new(None));
        let sink = result.clone();
        let running = exec.run(sh("printf hello; exit 3"), Box::new(move |info| *sink.borrow_mut() = Some(info)));
        assert!(running.as_ref().is_some_and(|r| r.pid().is_some()));
        pump(&mut event_loop, || result.borrow().is_some());
        let info = result.borrow().clone().expect("finished");
        assert_eq!((info.code, info.stdout.as_str()), (Some(3), "hello"));
    }

    #[test]
    fn run_applies_the_environment_and_the_working_directory() {
        let mut event_loop: TestLoop = EventLoop::try_new().unwrap();
        let exec = ProcessExec::shared(&event_loop.handle());
        let result = Rc::new(RefCell::new(None));
        let sink = result.clone();
        let cmd = sh("printf '%s %s' \"$SERP_X\" \"$(pwd)\"").env("SERP_X", "v").cwd("/");
        let _running = exec.run(cmd, Box::new(move |info| *sink.borrow_mut() = Some(info)));
        pump(&mut event_loop, || result.borrow().is_some());
        assert_eq!(result.borrow().as_ref().unwrap().stdout, "v /");
    }

    #[test]
    fn a_missing_program_fails_at_once_without_a_handle() {
        let event_loop: TestLoop = EventLoop::try_new().unwrap();
        let exec = ProcessExec::shared(&event_loop.handle());
        let result = Rc::new(RefCell::new(None));
        let sink = result.clone();
        let running =
            exec.run(Cmd::new(["serp-no-such-program"]), Box::new(move |info| *sink.borrow_mut() = Some(info)));
        assert!(running.is_none());
        assert!(!result.borrow().clone().expect("immediate failure").success());
        let empty = exec.run(Cmd::default(), Box::new(|info| assert!(!info.success())));
        assert!(empty.is_none());
    }

    #[test]
    fn the_timeout_kills_a_slow_process() {
        let mut event_loop: TestLoop = EventLoop::try_new().unwrap();
        let exec = ProcessExec::shared(&event_loop.handle());
        let result = Rc::new(RefCell::new(None));
        let sink = result.clone();
        let cmd = sh("sleep 30").timeout(Duration::from_millis(50));
        let _running = exec.run(cmd, Box::new(move |info| *sink.borrow_mut() = Some(info)));
        pump(&mut event_loop, || result.borrow().is_some());
        let info = result.borrow().clone().expect("killed by the timeout");
        assert!(!info.success());
        assert!(info.signal.is_some());
    }

    #[test]
    fn kill_ends_a_running_command_and_reports_it() {
        let mut event_loop: TestLoop = EventLoop::try_new().unwrap();
        let exec = ProcessExec::shared(&event_loop.handle());
        let result = Rc::new(RefCell::new(None));
        let sink = result.clone();
        let running = exec.run(sh("sleep 30"), Box::new(move |info| *sink.borrow_mut() = Some(info))).unwrap();
        running.kill();
        pump(&mut event_loop, || result.borrow().is_some());
        assert!(result.borrow().as_ref().unwrap().signal.is_some());
    }

    #[test]
    fn spawn_streams_lines_and_dropping_the_handle_stops_the_process() {
        let mut event_loop: TestLoop = EventLoop::try_new().unwrap();
        let exec = ProcessExec::shared(&event_loop.handle());
        let lines = Rc::new(RefCell::new(Vec::new()));
        let exited = Rc::new(Cell::new(false));
        let (sink, flag) = (lines.clone(), exited.clone());
        let running = exec
            .spawn(
                sh("echo one; echo two; sleep 30"),
                Box::new(move |line| sink.borrow_mut().push(line.to_owned())),
                Box::new(move |_| flag.set(true)),
            )
            .unwrap();
        pump(&mut event_loop, || lines.borrow().len() == 2);
        assert_eq!(*lines.borrow(), ["one", "two"]);
        assert!(!exited.get());
        drop(running);
        pump(&mut event_loop, || exited.get());
        assert!(exited.get());
    }

    #[test]
    fn detached_runs_without_a_handle() {
        let event_loop: TestLoop = EventLoop::try_new().unwrap();
        let exec = ProcessExec::shared(&event_loop.handle());
        let dir = tempfile::tempdir().unwrap();
        let marker = dir.path().join("ran");
        exec.detached(sh(&format!("touch {}", marker.display())));
        let deadline = Instant::now() + Duration::from_secs(5);
        while !marker.exists() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(marker.exists());
    }

    #[test]
    fn program_lookup_follows_the_path() {
        let event_loop: TestLoop = EventLoop::try_new().unwrap();
        let exec = ProcessExec::shared(&event_loop.handle());
        assert!(exec.has_program("sh"));
        assert!(!exec.has_program("serp-no-such-program"));
        assert!(exec.has_program("/bin/sh") || exec.has_program("/usr/bin/sh"));
    }
}
