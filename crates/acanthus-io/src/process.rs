use std::cell::RefCell;
use std::ffi::OsString;
use std::os::fd::{AsFd, BorrowedFd, OwnedFd};
use std::os::unix::process::{CommandExt, ExitStatusExt};
use std::path::PathBuf;
use std::process::{Child, Command, ExitStatus, Stdio};
use std::rc::Rc;

use acanthus_core::Signal as Reactive;
use calloop::generic::Generic;
use calloop::{Interest, LoopHandle, Mode, PostAction, RegistrationToken};
use rustix::fs::{OFlags, fcntl_getfl, fcntl_setfl};
use rustix::process::{Pid, PidfdFlags, Signal, kill_process_group, pidfd_open};

use crate::error::IoError;
use crate::weak::WeakHandle;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
/// How a child process ended.
pub struct ExitInfo {
    /// Exit code, if it exited normally.
    pub code: Option<i32>,
    /// Terminating signal, if any.
    pub signal: Option<i32>,
    /// Collected standard output; empty unless `collect_output` was set.
    pub stdout: String,
    /// Collected standard error; empty unless `collect_output` was set.
    pub stderr: String,
}

impl ExitInfo {
    /// Whether the process exited with code 0.
    pub fn success(&self) -> bool {
        self.code == Some(0)
    }

    fn new(status: ExitStatus, stdout: String, stderr: String) -> Self {
        ExitInfo { code: status.code(), signal: status.signal(), stdout, stderr }
    }
}

#[derive(Clone)]
struct SharedFd(Rc<OwnedFd>);

impl AsFd for SharedFd {
    fn as_fd(&self) -> BorrowedFd<'_> {
        self.0.as_fd()
    }
}

type LineCallback = Box<dyn FnMut(&str)>;
type ExitCallback = Box<dyn FnOnce(ExitInfo)>;

#[derive(Default)]
struct Stream {
    fd: Option<SharedFd>,
    pending: Vec<u8>,
    collected: String,
}

struct Shared<D: 'static> {
    handle: WeakHandle<D>,
    child: RefCell<Option<Child>>,
    pid: i32,
    out: RefCell<Stream>,
    err: RefCell<Stream>,
    collect: bool,
    tokens: RefCell<Vec<RegistrationToken>>,
    on_stdout_line: RefCell<Option<LineCallback>>,
    on_stderr_line: RefCell<Option<LineCallback>>,
    on_exit: RefCell<Option<ExitCallback>>,
    running: Reactive<bool>,
}

/// Builder for a child process run on the event loop, with line callbacks for its output.
pub struct Process {
    program: OsString,
    args: Vec<OsString>,
    env: Vec<(OsString, OsString)>,
    cwd: Option<PathBuf>,
    collect: bool,
    kill_on_drop: bool,
    on_stdout_line: Option<LineCallback>,
    on_stderr_line: Option<LineCallback>,
    on_exit: Option<ExitCallback>,
}

/// Handle to a spawned process; kills it on drop unless detached.
pub struct ProcessHandle {
    pid: i32,
    running: Reactive<bool>,
    kill: Rc<dyn Fn()>,
    kill_on_drop: bool,
}

impl ProcessHandle {
    /// Process id.
    pub fn pid(&self) -> i32 {
        self.pid
    }

    /// Signal that is true while the process runs.
    pub fn running(&self) -> Reactive<bool> {
        self.running.clone()
    }

    /// Sends SIGTERM to the process group.
    pub fn kill(&self) {
        (self.kill)();
    }

    /// Lets the process outlive the handle.
    pub fn detach(mut self) {
        self.kill_on_drop = false;
    }
}

impl Drop for ProcessHandle {
    fn drop(&mut self) {
        if self.kill_on_drop && self.running.get_untracked() {
            (self.kill)();
        }
    }
}

impl std::fmt::Debug for ProcessHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ProcessHandle").field("pid", &self.pid).finish()
    }
}

impl Process {
    /// Starts a builder for `program`.
    pub fn new(program: impl Into<OsString>) -> Self {
        Process {
            program: program.into(),
            args: Vec::new(),
            env: Vec::new(),
            cwd: None,
            collect: false,
            kill_on_drop: true,
            on_stdout_line: None,
            on_stderr_line: None,
            on_exit: None,
        }
    }

    /// Appends an argument.
    pub fn arg(mut self, arg: impl Into<OsString>) -> Self {
        self.args.push(arg.into());
        self
    }

    /// Appends several arguments.
    pub fn args<I, S>(mut self, args: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<OsString>,
    {
        self.args.extend(args.into_iter().map(Into::into));
        self
    }

    /// Sets an environment variable.
    pub fn env(mut self, key: impl Into<OsString>, value: impl Into<OsString>) -> Self {
        self.env.push((key.into(), value.into()));
        self
    }

    /// Sets the working directory.
    pub fn cwd(mut self, dir: impl Into<PathBuf>) -> Self {
        self.cwd = Some(dir.into());
        self
    }

    /// Collects stdout and stderr into the [`ExitInfo`] given to `on_exit`.
    pub fn collect_output(mut self) -> Self {
        self.collect = true;
        self
    }

    /// Sets whether dropping the handle kills the process, true by default.
    pub fn kill_on_drop(mut self, kill: bool) -> Self {
        self.kill_on_drop = kill;
        self
    }

    /// Calls `f` for every line of standard output.
    pub fn on_stdout_line(mut self, f: impl FnMut(&str) + 'static) -> Self {
        self.on_stdout_line = Some(Box::new(f));
        self
    }

    /// Calls `f` for every line of standard error.
    pub fn on_stderr_line(mut self, f: impl FnMut(&str) + 'static) -> Self {
        self.on_stderr_line = Some(Box::new(f));
        self
    }

    /// Calls `f` once when the process ends.
    pub fn on_exit(mut self, f: impl FnOnce(ExitInfo) + 'static) -> Self {
        self.on_exit = Some(Box::new(f));
        self
    }

    /// Spawns the process in its own process group and registers its pipes on the loop.
    pub fn spawn<D: 'static>(self, handle: &LoopHandle<'static, D>) -> Result<ProcessHandle, IoError> {
        let mut command = Command::new(&self.program);
        command.args(&self.args).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped()).process_group(0);
        for (k, v) in &self.env {
            command.env(k, v);
        }
        if let Some(dir) = &self.cwd {
            command.current_dir(dir);
        }
        let mut child = command.spawn()?;
        let pid = child.id() as i32;
        let stdout = child.stdout.take().map(|s| to_shared(OwnedFd::from(s))).transpose()?;
        let stderr = child.stderr.take().map(|s| to_shared(OwnedFd::from(s))).transpose()?;
        let pidfd = pidfd_open(Pid::from_raw(pid).expect("child pid"), PidfdFlags::empty())?;
        let running = Reactive::new(true);

        let shared = Rc::new(Shared {
            handle: WeakHandle::new(handle),
            child: RefCell::new(Some(child)),
            pid,
            out: RefCell::new(Stream { fd: stdout.clone(), ..Default::default() }),
            err: RefCell::new(Stream { fd: stderr.clone(), ..Default::default() }),
            collect: self.collect,
            tokens: RefCell::new(Vec::new()),
            on_stdout_line: RefCell::new(self.on_stdout_line),
            on_stderr_line: RefCell::new(self.on_stderr_line),
            on_exit: RefCell::new(self.on_exit),
            running: running.clone(),
        });

        for (fd, is_stdout) in [(stdout, true), (stderr, false)] {
            let Some(fd) = fd else { continue };
            let s = shared.clone();
            let token = handle.insert_source(Generic::new(fd, Interest::READ, Mode::Level), move |_, _, _| {
                s.pump(is_stdout);
                Ok(PostAction::Continue)
            })?;
            shared.tokens.borrow_mut().push(token);
        }
        let s = shared.clone();
        let token = handle.insert_source(Generic::new(pidfd, Interest::READ, Mode::Level), move |_, _, _| {
            s.reap();
            Ok(PostAction::Remove)
        })?;
        shared.tokens.borrow_mut().push(token);

        let killer = shared.clone();
        Ok(ProcessHandle { pid, running, kill: Rc::new(move || killer.terminate()), kill_on_drop: self.kill_on_drop })
    }
}

fn to_shared(fd: OwnedFd) -> Result<SharedFd, IoError> {
    let flags = fcntl_getfl(&fd)?;
    fcntl_setfl(&fd, flags | OFlags::NONBLOCK)?;
    Ok(SharedFd(Rc::new(fd)))
}

enum Fill {
    Open,
    Closed,
}

fn read_available(fd: &SharedFd, into: &mut Vec<u8>) -> Fill {
    let mut chunk = [0u8; 8192];
    loop {
        match rustix::io::read(fd, &mut chunk) {
            Ok(0) => return Fill::Closed,
            Ok(n) => into.extend_from_slice(&chunk[..n]),
            Err(rustix::io::Errno::INTR) => continue,
            Err(rustix::io::Errno::AGAIN) => return Fill::Open,
            Err(_) => return Fill::Closed,
        }
    }
}

fn split_lines(pending: &mut Vec<u8>, flush: bool, mut emit: impl FnMut(&str)) {
    while let Some(i) = pending.iter().position(|b| *b == b'\n') {
        let line: Vec<u8> = pending.drain(..=i).collect();
        let text = String::from_utf8_lossy(&line[..line.len() - 1]);
        emit(text.strip_suffix('\r').unwrap_or(&text));
    }
    if flush && !pending.is_empty() {
        let line = std::mem::take(pending);
        emit(&String::from_utf8_lossy(&line));
    }
}

impl<D: 'static> Shared<D> {
    fn pump(&self, is_stdout: bool) {
        self.pump_stream(is_stdout, false);
    }

    fn pump_stream(&self, is_stdout: bool, flush: bool) {
        let (stream, callback) =
            if is_stdout { (&self.out, &self.on_stdout_line) } else { (&self.err, &self.on_stderr_line) };
        let Some(fd) = stream.borrow().fd.clone() else { return };
        let mut chunk = Vec::new();
        let fill = read_available(&fd, &mut chunk);
        let mut lines: Vec<String> = Vec::new();
        {
            let mut stream = stream.borrow_mut();
            if self.collect {
                stream.collected.push_str(&String::from_utf8_lossy(&chunk));
            }
            stream.pending.extend_from_slice(&chunk);
            let wants_lines = callback.borrow().is_some();
            if wants_lines {
                split_lines(&mut stream.pending, flush || matches!(fill, Fill::Closed), |l| lines.push(l.to_owned()));
            } else {
                stream.pending.clear();
            }
        }
        if let Some(cb) = callback.borrow_mut().as_mut() {
            for line in &lines {
                cb(line);
            }
        }
    }

    fn reap(&self) {
        let status = self.child.borrow_mut().as_mut().and_then(|c| c.wait().ok());
        self.child.borrow_mut().take();
        self.pump_stream(true, true);
        self.pump_stream(false, true);
        let tokens: Vec<RegistrationToken> = self.tokens.borrow_mut().drain(..).collect();
        for token in tokens {
            self.handle.remove(token);
        }
        let out = std::mem::take(&mut self.out.borrow_mut().collected);
        let err = std::mem::take(&mut self.err.borrow_mut().collected);
        let exit = status.map(|s| ExitInfo::new(s, out, err)).unwrap_or_default();
        self.running.set(false);
        let callback = self.on_exit.borrow_mut().take();
        if let Some(cb) = callback {
            cb(exit);
        }
    }

    fn terminate(&self) {
        if let Some(pid) = Pid::from_raw(self.pid) {
            let _ = kill_process_group(pid, Signal::TERM);
        }
    }
}

/// Starts `argv` in its own process group with all streams closed and reaps it in the background.
pub fn exec_detached<I, S>(argv: I) -> Result<(), IoError>
where
    I: IntoIterator<Item = S>,
    S: Into<OsString>,
{
    let mut argv = argv.into_iter().map(Into::into);
    let program = argv.next().ok_or_else(|| IoError::Ipc("empty command".into()))?;
    let mut child = Command::new(program)
        .args(argv)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .process_group(0)
        .spawn()?;
    std::thread::spawn(move || {
        let _ = child.wait();
    });
    Ok(())
}

/// Runs `argv` to completion, blocking, and returns its exit status and output.
pub fn run_to_completion<I, S>(argv: I) -> Result<ExitInfo, IoError>
where
    I: IntoIterator<Item = S>,
    S: Into<OsString>,
{
    let mut argv = argv.into_iter().map(Into::into);
    let program = argv.next().ok_or_else(|| IoError::Ipc("empty command".into()))?;
    let output = Command::new(program).args(argv).stdin(Stdio::null()).output()?;
    Ok(ExitInfo {
        code: output.status.code(),
        signal: output.status.signal(),
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
    })
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use calloop::EventLoop;

    use super::*;

    fn run_until(event_loop: &mut EventLoop<'static, ()>, done: impl Fn() -> bool) {
        let deadline = Instant::now() + Duration::from_secs(5);
        while !done() && Instant::now() < deadline {
            event_loop.dispatch(Duration::from_millis(20), &mut ()).unwrap();
        }
    }

    #[test]
    fn streams_lines_and_reports_exit() {
        let mut event_loop: EventLoop<'static, ()> = EventLoop::try_new().unwrap();
        let lines = Rc::new(RefCell::new(Vec::new()));
        let errs = Rc::new(RefCell::new(Vec::new()));
        let exit = Rc::new(RefCell::new(None));
        let (l, e, x) = (lines.clone(), errs.clone(), exit.clone());
        let handle = Process::new("sh")
            .args(["-c", "printf 'a\\nb\\r\\nlast'; echo oops >&2; exit 3"])
            .on_stdout_line(move |s| l.borrow_mut().push(s.to_owned()))
            .on_stderr_line(move |s| e.borrow_mut().push(s.to_owned()))
            .on_exit(move |info| *x.borrow_mut() = Some(info))
            .spawn(&event_loop.handle())
            .unwrap();
        let running = handle.running();
        assert!(running.get_untracked());
        run_until(&mut event_loop, || exit.borrow().is_some());
        assert_eq!(*lines.borrow(), ["a", "b", "last"]);
        assert_eq!(*errs.borrow(), ["oops"]);
        assert_eq!(exit.borrow().as_ref().unwrap().code, Some(3));
        assert!(!running.get_untracked());
    }

    #[test]
    fn collects_output() {
        let mut event_loop: EventLoop<'static, ()> = EventLoop::try_new().unwrap();
        let exit = Rc::new(RefCell::new(None));
        let x = exit.clone();
        let _handle = Process::new("sh")
            .args(["-c", "echo hello; echo bad >&2"])
            .collect_output()
            .on_exit(move |info| *x.borrow_mut() = Some(info))
            .spawn(&event_loop.handle())
            .unwrap();
        run_until(&mut event_loop, || exit.borrow().is_some());
        let info = exit.borrow().clone().unwrap();
        assert!(info.success());
        assert_eq!(info.stdout, "hello\n");
        assert_eq!(info.stderr, "bad\n");
    }

    #[test]
    fn kill_terminates_the_process() {
        let mut event_loop: EventLoop<'static, ()> = EventLoop::try_new().unwrap();
        let exit = Rc::new(RefCell::new(None));
        let x = exit.clone();
        let handle = Process::new("sleep")
            .arg("30")
            .on_exit(move |info| *x.borrow_mut() = Some(info))
            .spawn(&event_loop.handle())
            .unwrap();
        handle.kill();
        run_until(&mut event_loop, || exit.borrow().is_some());
        let info = exit.borrow().clone().unwrap();
        assert_eq!(info.signal, Some(15));
        assert!(!handle.running().get_untracked());
    }

    #[test]
    fn missing_program_is_an_error() {
        let event_loop: EventLoop<'static, ()> = EventLoop::try_new().unwrap();
        assert!(Process::new("definitely-not-a-real-program").spawn(&event_loop.handle()).is_err());
    }

    #[test]
    fn exec_detached_runs_and_run_to_completion_captures() {
        exec_detached(["true"]).unwrap();
        let info = run_to_completion(["sh", "-c", "echo hi"]).unwrap();
        assert_eq!(info.stdout, "hi\n");
    }
}
