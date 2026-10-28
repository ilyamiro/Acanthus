use std::cell::{Cell, RefCell};
use std::collections::VecDeque;
use std::ffi::{CStr, CString, c_char, c_int, c_void};
use std::fs::File;
use std::io::{Read, Write};
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::path::Path;
use std::rc::Rc;
use std::sync::{Arc, Mutex, OnceLock};

use acanthus_core::{Signal, batch};
use acanthus_io::{IoError, WeakHandle};
use calloop::LoopHandle;
use calloop::channel::{Channel, Event, Sender, channel};

/// The PAM service used when none is set.
pub const DEFAULT_CONFIG: &str = "login";
/// The directory PAM service files are read from unless set otherwise.
pub const DEFAULT_CONFIG_DIRECTORY: &str = "/etc/pam.d";

const LIBRARY_NAMES: [&str; 2] = ["libpam.so.0", "libpam.so"];

const PAM_SUCCESS: c_int = 0;
const PAM_BUF_ERR: c_int = 5;
const PAM_AUTH_ERR: c_int = 7;
const PAM_MAXTRIES: c_int = 11;
const PAM_CONV_ERR: c_int = 19;

const PAM_PROMPT_ECHO_OFF: c_int = 1;
const PAM_PROMPT_ECHO_ON: c_int = 2;
const PAM_ERROR_MSG: c_int = 3;

const SIGKILL: c_int = 9;
const ERANGE: c_int = 34;

const TAG_REQUEST: u8 = 1;
const TAG_EXIT: u8 = 2;

const EXIT_SUCCESS: u8 = 0;
const EXIT_AUTH_FAILED: u8 = 1;
const EXIT_START_FAILED: u8 = 2;
const EXIT_MAX_TRIES: u8 = 3;
const EXIT_PAM_ERROR: u8 = 4;
const EXIT_OTHER: u8 = 5;

#[repr(C)]
struct Passwd {
    name: *mut c_char,
    password: *mut c_char,
    uid: u32,
    gid: u32,
    gecos: *mut c_char,
    directory: *mut c_char,
    shell: *mut c_char,
}

unsafe extern "C" {
    fn calloc(count: usize, size: usize) -> *mut c_void;
    fn malloc(size: usize) -> *mut c_void;
    fn pipe(fds: *mut c_int) -> c_int;
    fn close(fd: c_int) -> c_int;
    fn fork() -> c_int;
    fn kill(pid: c_int, signal: c_int) -> c_int;
    fn waitpid(pid: c_int, status: *mut c_int, options: c_int) -> c_int;
    fn _exit(code: c_int) -> !;
    fn getpwuid_r(uid: u32, entry: *mut Passwd, buffer: *mut c_char, size: usize, out: *mut *mut Passwd) -> c_int;
    fn getpwnam_r(
        name: *const c_char,
        entry: *mut Passwd,
        buffer: *mut c_char,
        size: usize,
        out: *mut *mut Passwd,
    ) -> c_int;
}

/// How a PAM conversation ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PamResult {
    /// Authentication succeeded.
    Success,
    /// Authentication failed.
    Failed,
    /// Something went wrong; see the preceding [`PamEvent::Error`].
    Error,
    /// Too many failed attempts.
    MaxTries,
}

/// What went wrong in a PAM conversation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PamError {
    /// The PAM helper could not be started.
    StartFailed,
    /// PAM reported an error while authenticating.
    TryAuthFailed,
    /// The helper failed in an unexpected way.
    InternalError,
}

/// An event of a PAM conversation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PamEvent {
    /// The message or prompt state changed.
    Message,
    /// The conversation ended with this result.
    Completed(PamResult),
    /// An error occurred.
    Error(PamError),
}

/// Maps a PAM return code to the exit code the helper reports.
pub fn classify(code: c_int) -> u8 {
    match code {
        PAM_SUCCESS => EXIT_SUCCESS,
        PAM_AUTH_ERR => EXIT_AUTH_FAILED,
        PAM_MAXTRIES => EXIT_MAX_TRIES,
        _ => EXIT_PAM_ERROR,
    }
}

/// The events produced by a helper exit code.
pub fn outcome(code: u8) -> Vec<PamEvent> {
    match code {
        EXIT_SUCCESS => vec![PamEvent::Completed(PamResult::Success)],
        EXIT_AUTH_FAILED => vec![PamEvent::Completed(PamResult::Failed)],
        EXIT_MAX_TRIES => vec![PamEvent::Completed(PamResult::MaxTries)],
        EXIT_START_FAILED => vec![PamEvent::Error(PamError::StartFailed), PamEvent::Completed(PamResult::Error)],
        EXIT_PAM_ERROR => vec![PamEvent::Error(PamError::TryAuthFailed), PamEvent::Completed(PamResult::Error)],
        _ => vec![PamEvent::Error(PamError::InternalError), PamEvent::Completed(PamResult::Error)],
    }
}

#[repr(C)]
struct RawMessage {
    style: c_int,
    text: *const c_char,
}

#[repr(C)]
struct RawResponse {
    text: *mut c_char,
    code: c_int,
}

type ConversationFn = unsafe extern "C" fn(c_int, *mut *const RawMessage, *mut *mut RawResponse, *mut c_void) -> c_int;

#[repr(C)]
struct RawConversation {
    function: ConversationFn,
    data: *mut c_void,
}

type StartFn = unsafe extern "C" fn(*const c_char, *const c_char, *const RawConversation, *mut *mut c_void) -> c_int;
type StartConfdirFn = unsafe extern "C" fn(
    *const c_char,
    *const c_char,
    *const RawConversation,
    *const c_char,
    *mut *mut c_void,
) -> c_int;
type EndFn = unsafe extern "C" fn(*mut c_void, c_int) -> c_int;
type StepFn = unsafe extern "C" fn(*mut c_void, c_int) -> c_int;

struct Library {
    start: StartFn,
    start_confdir: Option<StartConfdirFn>,
    end: EndFn,
    authenticate: StepFn,
    _library: libloading::Library,
}

fn load_library() -> Option<Library> {
    for name in LIBRARY_NAMES {
        let Ok(library) = (unsafe { libloading::Library::new(name) }) else { continue };
        let resolved = unsafe {
            let start = library.get::<StartFn>(b"pam_start\0").ok().map(|s| *s);
            let start_confdir = library.get::<StartConfdirFn>(b"pam_start_confdir\0").ok().map(|s| *s);
            let end = library.get::<EndFn>(b"pam_end\0").ok().map(|s| *s);
            let authenticate = library.get::<StepFn>(b"pam_authenticate\0").ok().map(|s| *s);
            start.zip(end).zip(authenticate).map(|((s, e), a)| (s, start_confdir, e, a))
        };
        if let Some((start, start_confdir, end, authenticate)) = resolved {
            return Some(Library { start, start_confdir, end, authenticate, _library: library });
        }
    }
    None
}

fn library() -> Option<&'static Library> {
    static LIBRARY: OnceLock<Option<Library>> = OnceLock::new();
    LIBRARY.get_or_init(load_library).as_ref()
}

/// True when `libpam` can be loaded.
pub fn pam_available() -> bool {
    library().is_some()
}

fn write_string(out: &mut Vec<u8>, text: &str) {
    out.extend_from_slice(&(text.len() as u32).to_le_bytes());
    out.extend_from_slice(text.as_bytes());
}

fn read_string(source: &mut impl Read) -> Option<String> {
    let mut length = [0u8; 4];
    source.read_exact(&mut length).ok()?;
    let mut data = vec![0u8; u32::from_le_bytes(length) as usize];
    source.read_exact(&mut data).ok()?;
    Some(String::from_utf8_lossy(&data).into_owned())
}

struct ChildIo {
    out: File,
    input: File,
}

fn duplicate(text: &str) -> *mut c_char {
    let bytes: Vec<u8> = text.bytes().filter(|b| *b != 0).collect();
    let pointer = unsafe { malloc(bytes.len() + 1) } as *mut u8;
    if pointer.is_null() {
        return std::ptr::null_mut();
    }
    unsafe {
        std::ptr::copy_nonoverlapping(bytes.as_ptr(), pointer, bytes.len());
        *pointer.add(bytes.len()) = 0;
    }
    pointer as *mut c_char
}

unsafe extern "C" fn converse(
    count: c_int,
    messages: *mut *const RawMessage,
    out: *mut *mut RawResponse,
    data: *mut c_void,
) -> c_int {
    if count <= 0 || messages.is_null() || out.is_null() || data.is_null() {
        return PAM_CONV_ERR;
    }
    let io = unsafe { &mut *(data as *mut ChildIo) };
    let total = count as usize;
    let responses = unsafe { calloc(total, std::mem::size_of::<RawResponse>()) } as *mut RawResponse;
    if responses.is_null() {
        return PAM_BUF_ERR;
    }
    for index in 0..total {
        let message = unsafe { *messages.add(index) };
        if message.is_null() {
            unsafe { _exit(EXIT_OTHER as c_int) };
        }
        let message = unsafe { &*message };
        let text = if message.text.is_null() {
            String::new()
        } else {
            unsafe { CStr::from_ptr(message.text) }.to_string_lossy().into_owned()
        };
        let needs_response = message.style == PAM_PROMPT_ECHO_OFF || message.style == PAM_PROMPT_ECHO_ON;
        let mut frame = vec![TAG_REQUEST, (message.style != PAM_PROMPT_ECHO_OFF) as u8];
        frame.push((message.style == PAM_ERROR_MSG) as u8);
        frame.push(needs_response as u8);
        write_string(&mut frame, &text);
        if io.out.write_all(&frame).is_err() {
            unsafe { _exit(1) };
        }
        if needs_response {
            let Some(answer) = read_string(&mut io.input) else { unsafe { _exit(EXIT_OTHER as c_int) } };
            let copy = duplicate(&answer);
            if copy.is_null() {
                unsafe { _exit(EXIT_OTHER as c_int) };
            }
            unsafe { (*responses.add(index)).text = copy };
        }
    }
    unsafe { *out = responses };
    PAM_SUCCESS
}

struct Request {
    service: CString,
    directory: CString,
    user: CString,
}

fn child_main(request: Request, out: File, input: File) -> u8 {
    let Some(library) = library() else { return EXIT_START_FAILED };
    let mut io = ChildIo { out, input };
    let raw = RawConversation { function: converse, data: &mut io as *mut ChildIo as *mut c_void };
    let mut handle: *mut c_void = std::ptr::null_mut();
    let started = unsafe {
        match library.start_confdir {
            Some(start) => {
                start(request.service.as_ptr(), request.user.as_ptr(), &raw, request.directory.as_ptr(), &mut handle)
            }
            None => (library.start)(request.service.as_ptr(), request.user.as_ptr(), &raw, &mut handle),
        }
    };
    if started != PAM_SUCCESS {
        return EXIT_START_FAILED;
    }
    let result = unsafe { (library.authenticate)(handle, 0) };
    unsafe { (library.end)(handle, result) };
    classify(result)
}

fn close_inherited(keep: &[c_int]) {
    let limit = rustix::process::getrlimit(rustix::process::Resource::Nofile).current.unwrap_or(1024).min(65536);
    for fd in 3..limit as c_int {
        if !keep.contains(&fd) {
            unsafe { close(fd) };
        }
    }
}

fn spawn_child(request: Request) -> Option<(i32, File, File)> {
    let mut to_child = [0 as c_int; 2];
    let mut from_child = [0 as c_int; 2];
    if unsafe { pipe(to_child.as_mut_ptr()) } != 0 {
        return None;
    }
    if unsafe { pipe(from_child.as_mut_ptr()) } != 0 {
        unsafe {
            drop(OwnedFd::from_raw_fd(to_child[0]));
            drop(OwnedFd::from_raw_fd(to_child[1]));
        }
        return None;
    }
    let files = unsafe {
        [
            File::from(OwnedFd::from_raw_fd(to_child[0])),
            File::from(OwnedFd::from_raw_fd(to_child[1])),
            File::from(OwnedFd::from_raw_fd(from_child[0])),
            File::from(OwnedFd::from_raw_fd(from_child[1])),
        ]
    };
    let [child_input, parent_out, parent_in, child_out] = files;
    let pid = unsafe { fork() };
    if pid < 0 {
        return None;
    }
    if pid == 0 {
        drop(parent_out);
        drop(parent_in);
        let mut exit_out = child_out.try_clone();
        let keep =
            [child_input.as_raw_fd(), child_out.as_raw_fd(), exit_out.as_ref().map(|f| f.as_raw_fd()).unwrap_or(-1)];
        close_inherited(&keep);
        let code =
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| child_main(request, child_out, child_input)))
                .unwrap_or(EXIT_OTHER);
        if let Ok(out) = exit_out.as_mut() {
            let _ = out.write_all(&[TAG_EXIT, code]);
        }
        unsafe { _exit(0) };
    }
    drop(child_input);
    drop(child_out);
    Some((pid, parent_out, parent_in))
}

enum Msg {
    Request { session: u64, text: String, error: bool, response_required: bool, echo: bool },
    Exit { session: u64, code: u8 },
}

fn read_events(session: u64, mut input: File, sink: Sender<Msg>, child: Arc<Mutex<Option<i32>>>) {
    loop {
        let mut tag = [0u8; 1];
        if input.read_exact(&mut tag).is_err() {
            let _ = sink.send(Msg::Exit { session, code: EXIT_OTHER });
            break;
        }
        match tag[0] {
            TAG_REQUEST => {
                let mut flags = [0u8; 3];
                let text = input.read_exact(&mut flags).ok().and_then(|_| read_string(&mut input));
                let Some(text) = text else {
                    let _ = sink.send(Msg::Exit { session, code: EXIT_OTHER });
                    break;
                };
                let request = Msg::Request {
                    session,
                    text,
                    error: flags[1] != 0,
                    response_required: flags[2] != 0,
                    echo: flags[0] != 0,
                };
                if sink.send(request).is_err() {
                    break;
                }
            }
            TAG_EXIT => {
                let mut code = [0u8; 1];
                let code = if input.read_exact(&mut code).is_ok() { code[0] } else { EXIT_OTHER };
                let _ = sink.send(Msg::Exit { session, code });
                break;
            }
            _ => {
                let _ = sink.send(Msg::Exit { session, code: EXIT_OTHER });
                break;
            }
        }
    }
    reap(&child, false);
}

fn reap(child: &Mutex<Option<i32>>, terminate: bool) {
    let pid = child.lock().ok().and_then(|mut pid| pid.take());
    if let Some(pid) = pid {
        unsafe {
            if terminate {
                kill(pid, SIGKILL);
            }
            waitpid(pid, std::ptr::null_mut(), 0);
        }
    }
}

fn lookup_user(name: Option<&str>) -> Option<String> {
    let mut size = 1024usize;
    let wanted = match name {
        Some(name) => Some(CString::new(name).ok()?),
        None => None,
    };
    loop {
        let mut buffer = vec![0 as c_char; size];
        let mut entry = Passwd {
            name: std::ptr::null_mut(),
            password: std::ptr::null_mut(),
            uid: 0,
            gid: 0,
            gecos: std::ptr::null_mut(),
            directory: std::ptr::null_mut(),
            shell: std::ptr::null_mut(),
        };
        let mut found: *mut Passwd = std::ptr::null_mut();
        let code = unsafe {
            match &wanted {
                Some(name) => getpwnam_r(name.as_ptr(), &mut entry, buffer.as_mut_ptr(), size, &mut found),
                None => {
                    getpwuid_r(rustix::process::getuid().as_raw(), &mut entry, buffer.as_mut_ptr(), size, &mut found)
                }
            }
        };
        if code == ERANGE && size < 1 << 20 {
            size *= 4;
            continue;
        }
        if code != 0 || found.is_null() || entry.name.is_null() {
            return None;
        }
        return Some(unsafe { CStr::from_ptr(entry.name) }.to_string_lossy().into_owned());
    }
}

/// Observable state of a PAM conversation.
#[derive(Clone)]
pub struct PamState {
    /// True while a conversation runs.
    pub active: Signal<bool>,
    /// The latest message or prompt text.
    pub message: Signal<String>,
    /// True when the message is an error.
    pub message_is_error: Signal<bool>,
    /// True when PAM waits for a response.
    pub response_required: Signal<bool>,
    /// True when the response may be shown while typed.
    pub response_visible: Signal<bool>,
}

impl PamState {
    fn new() -> Self {
        PamState {
            active: Signal::new(false),
            message: Signal::new(String::new()),
            message_is_error: Signal::new(false),
            response_required: Signal::new(false),
            response_visible: Signal::new(false),
        }
    }
}

struct Conversation {
    child: Arc<Mutex<Option<i32>>>,
    writer: File,
}

struct Inner {
    state: PamState,
    session: Cell<u64>,
    conversation: RefCell<Option<Conversation>>,
    on_event: RefCell<Box<dyn FnMut(PamEvent)>>,
    queue: RefCell<VecDeque<PamEvent>>,
    draining: Cell<bool>,
}

impl Inner {
    fn emit(&self, event: PamEvent) {
        self.queue.borrow_mut().push_back(event);
        if self.draining.replace(true) {
            return;
        }
        loop {
            let next = self.queue.borrow_mut().pop_front();
            let Some(event) = next else { break };
            (self.on_event.borrow_mut())(event);
        }
        self.draining.set(false);
    }

    fn end_conversation(&self) {
        self.session.set(self.session.get() + 1);
        let conversation = self.conversation.borrow_mut().take();
        if let Some(conversation) = conversation {
            reap(&conversation.child, true);
        }
        batch(|| {
            self.state.active.set(false);
            self.state.message.set(String::new());
            self.state.message_is_error.set(false);
            self.state.response_required.set(false);
        });
    }

    fn handle(&self, message: Msg) {
        match message {
            Msg::Request { session, text, error, response_required, echo } if session == self.session.get() => {
                batch(|| {
                    self.state.message.set(text);
                    self.state.message_is_error.set(error);
                    self.state.response_visible.set(echo);
                    self.state.response_required.set(response_required);
                });
                self.emit(PamEvent::Message);
            }
            Msg::Exit { session, code } if session == self.session.get() => {
                self.end_conversation();
                for event in outcome(code) {
                    self.emit(event);
                }
            }
            _ => {}
        }
    }
}

/// Authentication through PAM in a helper child process, so that a crash cannot take the application down. Messages and prompts arrive as signals and events.
pub struct PamContext {
    /// Its observable state.
    pub state: PamState,
    config: RefCell<String>,
    config_directory: RefCell<String>,
    user: RefCell<Option<String>>,
    inner: Rc<Inner>,
    sink: Sender<Msg>,
    remove: Option<Box<dyn FnOnce()>>,
}

impl PamContext {
    /// Creates a context delivering events to `on_event` on the loop behind `handle`.
    pub fn new<D: 'static>(
        handle: &LoopHandle<'static, D>,
        on_event: impl FnMut(PamEvent) + 'static,
    ) -> Result<PamContext, IoError> {
        let state = PamState::new();
        let inner = Rc::new(Inner {
            state: state.clone(),
            session: Cell::new(0),
            conversation: RefCell::new(None),
            on_event: RefCell::new(Box::new(on_event)),
            queue: RefCell::new(VecDeque::new()),
            draining: Cell::new(false),
        });
        let (sink, receiver): (Sender<Msg>, Channel<Msg>) = channel();
        let target = inner.clone();
        let token = handle.insert_source(receiver, move |event, _, _| {
            if let Event::Msg(message) = event {
                target.handle(message);
            }
        })?;
        let weak = WeakHandle::new(handle);
        Ok(PamContext {
            state,
            config: RefCell::new(DEFAULT_CONFIG.to_owned()),
            config_directory: RefCell::new(DEFAULT_CONFIG_DIRECTORY.to_owned()),
            user: RefCell::new(None),
            inner,
            sink,
            remove: Some(Box::new(move || weak.remove(token))),
        })
    }

    /// The PAM service name in use.
    pub fn config(&self) -> String {
        self.config.borrow().clone()
    }

    /// Sets the PAM service name; ignored while a conversation runs.
    pub fn set_config(&self, config: &str) {
        if !self.is_active() {
            *self.config.borrow_mut() = config.to_owned();
        }
    }

    /// The directory PAM service files are read from.
    pub fn config_directory(&self) -> String {
        self.config_directory.borrow().clone()
    }

    /// Sets the directory of PAM service files; ignored while a conversation runs.
    pub fn set_config_directory(&self, directory: &str) {
        if !self.is_active() {
            *self.config_directory.borrow_mut() = directory.to_owned();
        }
    }

    /// Sets the user to authenticate, or the current user for `None`; ignored while a conversation runs.
    pub fn set_user(&self, user: Option<&str>) {
        if !self.is_active() {
            *self.user.borrow_mut() = user.filter(|name| !name.is_empty()).map(str::to_owned);
        }
    }

    /// True while a conversation runs.
    pub fn is_active(&self) -> bool {
        self.state.active.get()
    }

    /// Starts a conversation; false when the configuration or user is missing or the helper cannot start. Returns true when one is already running.
    pub fn start(&self) -> bool {
        if self.is_active() {
            return true;
        }
        let directory = self.config_directory();
        if !Path::new(&directory).is_dir() {
            tracing::error!("pam config directory {directory} is not a directory");
            return false;
        }
        let service = self.config();
        if !Path::new(&directory).join(&service).is_file() {
            tracing::error!("pam config {service} is not a file in {directory}");
            return false;
        }
        let Some(user) = lookup_user(self.user.borrow().as_deref()) else {
            tracing::error!("pam user not found");
            return false;
        };
        let session = self.inner.session.get() + 1;
        self.inner.session.set(session);
        let (Ok(service), Ok(directory), Ok(user), true) =
            (CString::new(service), CString::new(directory), CString::new(user), pam_available())
        else {
            let _ = self.sink.send(Msg::Exit { session, code: EXIT_START_FAILED });
            return false;
        };
        let Some((pid, writer, reader)) = spawn_child(Request { service, directory, user }) else {
            let _ = self.sink.send(Msg::Exit { session, code: EXIT_OTHER });
            return false;
        };
        let child = Arc::new(Mutex::new(Some(pid)));
        *self.inner.conversation.borrow_mut() = Some(Conversation { child: child.clone(), writer });
        self.state.active.set(true);
        let sink = self.sink.clone();
        if std::thread::Builder::new()
            .name("acanthus-pam".into())
            .spawn(move || read_events(session, reader, sink, child))
            .is_err()
        {
            self.inner.end_conversation();
            return false;
        }
        true
    }

    /// Answers the pending prompt; false when none is pending.
    pub fn respond(&self, text: &str) -> bool {
        if !self.is_active() || !self.state.response_required.get() {
            return false;
        }
        let mut frame = Vec::new();
        write_string(&mut frame, text);
        let sent = self
            .inner
            .conversation
            .borrow_mut()
            .as_mut()
            .map(|conversation| conversation.writer.write_all(&frame).is_ok())
            .unwrap_or(false);
        if !sent {
            self.inner.end_conversation();
            for event in outcome(EXIT_OTHER) {
                self.inner.emit(event);
            }
        }
        sent
    }

    /// Ends the running conversation.
    pub fn abort(&self) {
        self.inner.end_conversation();
    }
}

impl Drop for PamContext {
    fn drop(&mut self) {
        self.inner.end_conversation();
        if let Some(remove) = self.remove.take() {
            remove();
        }
    }
}

/// True when `directory` holds a file for the PAM service `service`.
pub fn service_exists(directory: &Path, service: &str) -> bool {
    directory.join(service).exists()
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use calloop::EventLoop;

    use super::*;

    type Log = Rc<RefCell<Vec<PamEvent>>>;
    type Messages = Rc<RefCell<Vec<(String, bool, bool, bool)>>>;

    struct Rig {
        event_loop: EventLoop<'static, ()>,
        context: PamContext,
        log: Log,
        messages: Messages,
        dir: tempfile::TempDir,
    }

    fn rig() -> Rig {
        let event_loop: EventLoop<'static, ()> = EventLoop::try_new().unwrap();
        let log: Log = Rc::default();
        let messages: Messages = Rc::default();
        let slot: Rc<RefCell<Option<PamState>>> = Rc::default();
        let (sink, recorded, state_slot) = (log.clone(), messages.clone(), slot.clone());
        let context = PamContext::new(&event_loop.handle(), move |event| {
            sink.borrow_mut().push(event);
            let state = state_slot.borrow().clone();
            if let (PamEvent::Message, Some(state)) = (event, state) {
                recorded.borrow_mut().push((
                    state.message.get(),
                    state.message_is_error.get(),
                    state.response_required.get(),
                    state.response_visible.get(),
                ));
            }
        })
        .unwrap();
        *slot.borrow_mut() = Some(context.state.clone());
        let dir = tempfile::tempdir().unwrap();
        context.set_config_directory(dir.path().to_str().unwrap());
        context.set_user(Some("nobody"));
        Rig { event_loop, context, log, messages, dir }
    }

    impl Rig {
        fn service(&self, name: &str, body: &str) {
            std::fs::write(self.dir.path().join(name), body).unwrap();
            self.context.set_config(name);
        }

        fn pump_until(&mut self, mut done: impl FnMut(&Rig) -> bool) -> bool {
            let deadline = Instant::now() + Duration::from_secs(10);
            while !done(self) && Instant::now() < deadline {
                self.event_loop.dispatch(Duration::from_millis(10), &mut ()).unwrap();
            }
            done(self)
        }

        fn settle(&mut self) {
            let end = Instant::now() + Duration::from_millis(300);
            while Instant::now() < end {
                self.event_loop.dispatch(Duration::from_millis(10), &mut ()).unwrap();
            }
        }

        fn completed(&self) -> Option<PamResult> {
            self.log.borrow().iter().find_map(|event| match event {
                PamEvent::Completed(result) => Some(*result),
                _ => None,
            })
        }
    }

    fn require_library() -> bool {
        if pam_available() {
            true
        } else {
            eprintln!("libpam not found, skipping");
            false
        }
    }

    const PERMIT: &str = "auth required pam_permit.so\naccount required pam_permit.so\n";
    const PROMPT_SERVICE: &str =
        "auth optional pam_faildelay.so delay=0\nauth required pam_unix.so nullok\naccount required pam_permit.so\n";

    #[test]
    fn pam_codes_map_to_failures() {
        assert_eq!(outcome(classify(0)), [PamEvent::Completed(PamResult::Success)]);
        assert_eq!(outcome(classify(7)), [PamEvent::Completed(PamResult::Failed)]);
        assert_eq!(outcome(classify(11)), [PamEvent::Completed(PamResult::MaxTries)]);
        let error = [PamEvent::Error(PamError::TryAuthFailed), PamEvent::Completed(PamResult::Error)];
        for code in [9, 10, 6, 19] {
            assert_eq!(outcome(classify(code)), error);
        }
        assert_eq!(outcome(EXIT_START_FAILED)[0], PamEvent::Error(PamError::StartFailed));
        assert_eq!(outcome(EXIT_OTHER)[0], PamEvent::Error(PamError::InternalError));
    }

    #[test]
    fn user_lookup_resolves_the_current_and_named_users() {
        assert!(lookup_user(None).is_some());
        assert_eq!(lookup_user(Some("root")).as_deref(), Some("root"));
        assert_eq!(lookup_user(Some("no-such-user-xyz")), None);
    }

    #[test]
    fn permit_service_succeeds() {
        if !require_library() {
            return;
        }
        let mut rig = rig();
        rig.service("permit", PERMIT);
        assert!(rig.context.start());
        assert!(rig.context.is_active());
        assert!(rig.context.start());
        assert!(rig.pump_until(|r| r.completed().is_some()));
        assert_eq!(*rig.log.borrow(), [PamEvent::Completed(PamResult::Success)]);
        assert!(!rig.context.is_active());
    }

    #[test]
    fn deny_service_fails() {
        if !require_library() {
            return;
        }
        let mut rig = rig();
        rig.service("deny", "auth required pam_deny.so\naccount required pam_permit.so\n");
        assert!(rig.context.start());
        assert!(rig.pump_until(|r| r.completed().is_some()));
        assert_eq!(rig.completed(), Some(PamResult::Failed));
    }

    #[test]
    fn invalid_config_or_user_refuses_to_start_without_events() {
        let mut rig = rig();
        rig.context.set_config("does-not-exist");
        assert!(!rig.context.start());
        rig.service("permit", PERMIT);
        rig.context.set_user(Some("no-such-user-xyz"));
        assert!(!rig.context.start());
        rig.context.set_config_directory("/definitely/not/here");
        rig.context.set_user(None);
        assert!(!rig.context.start());
        rig.settle();
        assert!(rig.log.borrow().is_empty());
    }

    #[test]
    fn a_broken_stack_reports_error_then_completed_error() {
        if !require_library() {
            return;
        }
        let mut rig = rig();
        rig.service("broken", "auth required pam_nonexistent_module.so\n");
        assert!(rig.context.start());
        assert!(rig.pump_until(|r| r.completed().is_some()));
        assert_eq!(rig.completed(), Some(PamResult::Error));
        assert!(matches!(rig.log.borrow()[0], PamEvent::Error(PamError::TryAuthFailed | PamError::StartFailed)));
    }

    #[test]
    fn text_info_is_visible_and_needs_no_response() {
        if !require_library() {
            return;
        }
        let mut rig = rig();
        let motd = rig.dir.path().join("motd");
        std::fs::write(&motd, "Place your finger\n").unwrap();
        rig.service("info", &format!("auth optional pam_echo.so file={}\n{PERMIT}", motd.display()));
        assert!(rig.context.start());
        assert!(rig.pump_until(|r| r.completed().is_some()));
        assert_eq!(rig.completed(), Some(PamResult::Success));
        let messages = rig.messages.borrow();
        assert_eq!(messages.len(), 1);
        assert!(messages[0].0.contains("Place your finger"));
        assert_eq!((messages[0].1, messages[0].2, messages[0].3), (false, false, true));
        assert_eq!(rig.context.state.message.get(), "");
    }

    #[test]
    fn prompt_round_trip_reaches_the_answer() {
        if !require_library() {
            return;
        }
        let mut rig = rig();
        rig.service("prompt", PROMPT_SERVICE);
        assert!(rig.context.start());
        assert!(rig.pump_until(|r| r.context.state.response_required.get()));
        assert!(!rig.context.state.response_visible.get());
        assert!(!rig.context.state.message.get().is_empty());
        assert!(rig.context.respond("definitely not the password"));
        assert!(rig.pump_until(|r| r.completed().is_some()));
        assert!(matches!(rig.completed(), Some(PamResult::Failed | PamResult::Error)));
        assert!(!rig.context.is_active());
        assert!(!rig.context.respond("again"));
    }

    #[test]
    fn abort_kills_the_child_clears_state_and_emits_nothing() {
        if !require_library() {
            return;
        }
        let mut rig = rig();
        rig.service("prompt", PROMPT_SERVICE);
        assert!(rig.context.start());
        assert!(rig.pump_until(|r| r.context.state.response_required.get()));
        let before = rig.log.borrow().len();
        rig.context.abort();
        assert!(!rig.context.is_active());
        assert!(!rig.context.state.response_required.get());
        assert_eq!(rig.context.state.message.get(), "");
        rig.service("permit", PERMIT);
        assert!(rig.context.start());
        assert!(rig.pump_until(|r| r.completed().is_some()));
        assert_eq!(rig.log.borrow()[before..], [PamEvent::Completed(PamResult::Success)]);
    }

    #[test]
    fn a_blocking_module_is_killed_on_abort() {
        if !require_library() || !Path::new("/bin/sh").exists() {
            return;
        }
        let mut rig = rig();
        rig.service("slow", &format!("auth requisite pam_exec.so quiet /bin/sh -c 'sleep 30'\n{PERMIT}"));
        assert!(rig.context.start());
        rig.settle();
        let started = Instant::now();
        rig.context.abort();
        assert!(started.elapsed() < Duration::from_secs(2));
        assert!(!rig.context.is_active());
    }

    #[test]
    fn abort_from_inside_the_callback_does_not_deadlock() {
        if !require_library() {
            return;
        }
        let event_loop: EventLoop<'static, ()> = EventLoop::try_new().unwrap();
        let slot: Rc<RefCell<Option<Rc<PamContext>>>> = Rc::default();
        let seen: Log = Rc::default();
        let (inner_slot, inner_seen) = (slot.clone(), seen.clone());
        let context = Rc::new(
            PamContext::new(&event_loop.handle(), move |event| {
                inner_seen.borrow_mut().push(event);
                let context = inner_slot.borrow().clone();
                if let (PamEvent::Message, Some(context)) = (event, context) {
                    context.abort();
                }
            })
            .unwrap(),
        );
        *slot.borrow_mut() = Some(context.clone());
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("prompt"), PROMPT_SERVICE).unwrap();
        context.set_config_directory(dir.path().to_str().unwrap());
        context.set_config("prompt");
        context.set_user(Some("nobody"));
        assert!(context.start());
        let mut event_loop = event_loop;
        let deadline = Instant::now() + Duration::from_secs(10);
        while seen.borrow().is_empty() && Instant::now() < deadline {
            event_loop.dispatch(Duration::from_millis(10), &mut ()).unwrap();
        }
        event_loop.dispatch(Duration::from_millis(50), &mut ()).unwrap();
        assert_eq!(*seen.borrow(), [PamEvent::Message]);
        assert!(!context.is_active());
        slot.borrow_mut().take();
    }

    #[test]
    fn settings_are_frozen_while_active() {
        if !require_library() {
            return;
        }
        let mut rig = rig();
        rig.service("prompt", PROMPT_SERVICE);
        assert!(rig.context.start());
        rig.context.set_config("login");
        assert_eq!(rig.context.config(), "prompt");
        rig.context.abort();
        rig.context.set_config("login");
        assert_eq!(rig.context.config(), "login");
        rig.settle();
    }

    #[test]
    fn dropping_a_running_context_is_clean() {
        if !require_library() {
            return;
        }
        let rig = rig();
        rig.service("prompt", PROMPT_SERVICE);
        rig.context.start();
        let Rig { event_loop, context, .. } = rig;
        drop(context);
        let mut event_loop = event_loop;
        event_loop.dispatch(Duration::from_millis(20), &mut ()).unwrap();
    }
}
