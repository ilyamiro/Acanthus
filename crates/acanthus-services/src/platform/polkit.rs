use std::collections::{BTreeMap, HashMap, VecDeque};
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::process::{Command as Process, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use acanthus_core::{Signal, batch};
use acanthus_io::IoError;
use calloop::LoopHandle;
use futures_lite::StreamExt;
use futures_lite::future::{self, FutureExt};
use zbus::fdo::DBusProxy;
use zbus::{Connection, OwnedVariant, interface, proxy};
use zbus::{ObjectPath, Variant as Value};

use crate::platform::bridge::{EventSink, ServiceHandle, spawn_worker};
use crate::platform::dbus::BusKind;

const AUTHORITY_NAME: &str = "org.freedesktop.PolicyKit1";
fn agent_path(app: &str) -> String {
    format!("/org/{app}/PolicyKit1/AuthenticationAgent")
}
const HELPER_NAME: &str = "polkit-agent-helper-1";
const HELPER_SOCKET: &str = "/run/polkit/agent-helper.socket";
const HELPER_DIRS: [&str; 7] = [
    "/run/wrappers/bin",
    "/usr/lib/polkit-1",
    "/usr/libexec/polkit-1",
    "/usr/lib/polkit",
    "/usr/libexec",
    "/usr/lib/policykit-1",
    "/usr/lib64/polkit-1",
];
const RETRY_DELAY: Duration = Duration::from_secs(10);
const DEFAULT_LOCALE: &str = "en_US.UTF-8";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Transport {
    Socket(PathBuf),
    Spawn(PathBuf),
}

impl Transport {
    pub fn discover() -> Option<Transport> {
        let path_dirs: Vec<PathBuf> =
            std::env::var_os("PATH").map(|path| std::env::split_paths(&path).collect()).unwrap_or_default();
        Transport::discover_with(&path_dirs, |path| path.exists())
    }

    pub fn discover_with(path_dirs: &[PathBuf], exists: impl Fn(&Path) -> bool) -> Option<Transport> {
        let socket = Path::new(HELPER_SOCKET);
        if exists(socket) {
            return Some(Transport::Socket(socket.to_owned()));
        }
        HELPER_DIRS
            .iter()
            .map(PathBuf::from)
            .chain(path_dirs.iter().cloned())
            .map(|dir| dir.join(HELPER_NAME))
            .find(|candidate| exists(candidate))
            .map(Transport::Spawn)
    }
}

#[derive(Debug, Clone)]
pub struct PolkitOptions {
    pub transport: Option<Transport>,
    pub session_id: Option<String>,
    pub agent_path: String,
    pub locale: String,
    pub retry_delay: Duration,
    pub passwd: PathBuf,
    pub group: PathBuf,
    pub uid: u32,
}

impl PolkitOptions {
    pub fn for_app(app: &str) -> Self {
        let locale = ["LC_ALL", "LC_MESSAGES", "LANG"]
            .iter()
            .find_map(|name| std::env::var(name).ok().filter(|value| !value.is_empty()))
            .unwrap_or_else(|| DEFAULT_LOCALE.to_owned());
        PolkitOptions {
            transport: Transport::discover(),
            session_id: std::env::var("XDG_SESSION_ID").ok().filter(|id| !id.is_empty()),
            agent_path: agent_path(app),
            locale,
            retry_delay: RETRY_DELAY,
            passwd: PathBuf::from("/etc/passwd"),
            group: PathBuf::from("/etc/group"),
            uid: rustix::process::getuid().as_raw(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PolkitIdentity {
    pub uid: u32,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawIdentity {
    pub kind: String,
    pub id: Option<u32>,
}

pub fn parse_passwd(text: &str) -> Vec<(String, u32, u32)> {
    text.lines()
        .filter_map(|line| {
            let mut fields = line.split(':');
            let name = fields.next()?;
            let uid = fields.nth(1)?.parse().ok()?;
            let gid = fields.next()?.parse().ok()?;
            (!name.is_empty()).then(|| (name.to_owned(), uid, gid))
        })
        .collect()
}

pub fn parse_group(text: &str) -> Vec<(u32, Vec<String>)> {
    text.lines()
        .filter_map(|line| {
            let mut fields = line.split(':');
            let gid = fields.nth(2)?.parse().ok()?;
            let members = fields.next().unwrap_or("").split(',').filter(|m| !m.is_empty()).map(str::to_owned);
            Some((gid, members.collect()))
        })
        .collect()
}

pub fn resolve_identities(raw: &[RawIdentity], passwd: &str, group: &str) -> Vec<PolkitIdentity> {
    let users = parse_passwd(passwd);
    let groups = parse_group(group);
    let mut found: Vec<PolkitIdentity> = Vec::new();
    let mut add = |uid: u32, name: &str| {
        if !found.iter().any(|identity| identity.uid == uid) {
            found.push(PolkitIdentity { uid, name: name.to_owned() });
        }
    };
    for identity in raw {
        let Some(id) = identity.id else { continue };
        match identity.kind.as_str() {
            "unix-user" => {
                if let Some((name, uid, _)) = users.iter().find(|(_, uid, _)| *uid == id) {
                    add(*uid, name);
                }
            }
            "unix-group" => {
                for (name, uid, gid) in &users {
                    let member = groups.iter().any(|(g, members)| *g == id && members.contains(name));
                    if *gid == id || member {
                        add(*uid, name);
                    }
                }
            }
            _ => {}
        }
    }
    found
}

pub fn default_identity(identities: &[PolkitIdentity], uid: u32) -> usize {
    identities.iter().position(|identity| identity.uid == uid).unwrap_or(0)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LineResult {
    Prompt,
    Message,
    Success,
    Failure,
    Ignored,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PolkitFlow {
    pub cookie: String,
    pub action_id: String,
    pub message: String,
    pub icon_name: String,
    pub details: BTreeMap<String, String>,
    pub identities: Vec<PolkitIdentity>,
    pub selected: usize,
    pub response_required: bool,
    pub input_prompt: String,
    pub echo: bool,
    pub supplementary_message: String,
    pub supplementary_is_error: bool,
}

impl PolkitFlow {
    pub fn identity(&self) -> Option<&PolkitIdentity> {
        self.identities.get(self.selected)
    }

    fn reset_session(&mut self) {
        self.response_required = false;
        self.input_prompt.clear();
        self.echo = false;
        self.supplementary_message.clear();
        self.supplementary_is_error = false;
    }

    pub fn apply_line(&mut self, line: &str) -> LineResult {
        let line = line.trim_end_matches(['\r', '\n']);
        let (tag, rest) = line.split_once(' ').unwrap_or((line, ""));
        match tag {
            "PAM_PROMPT_ECHO_OFF" | "PAM_PROMPT_ECHO_ON" => {
                self.response_required = true;
                self.echo = tag == "PAM_PROMPT_ECHO_ON";
                self.input_prompt = rest.to_owned();
                LineResult::Prompt
            }
            "PAM_ERROR_MSG" | "PAM_TEXT_INFO" => {
                self.supplementary_message = rest.to_owned();
                self.supplementary_is_error = tag == "PAM_ERROR_MSG";
                LineResult::Message
            }
            "SUCCESS" => LineResult::Success,
            "FAILURE" => LineResult::Failure,
            _ => LineResult::Ignored,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PolkitError {
    Failed,
    Message(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PolkitEvent {
    RequestStarted,
    Cancelled,
    Failed,
    Succeeded,
}

#[derive(Debug)]
enum Outcome {
    Started,
    Failed(Option<String>),
    Succeeded,
    Cancelled,
}

#[derive(Debug)]
enum Event {
    Available(bool),
    Registered(bool),
    Flow(Option<PolkitFlow>),
    Outcome(Outcome),
}

#[derive(Clone)]
pub struct PolkitState {
    pub available: Signal<bool>,
    pub registered: Signal<bool>,
    pub flow: Signal<Option<PolkitFlow>>,
    pub error: Signal<Option<PolkitError>>,
}

impl PolkitState {
    fn new() -> Self {
        PolkitState {
            available: Signal::new(false),
            registered: Signal::new(false),
            flow: Signal::new(None),
            error: Signal::new(None),
        }
    }

    pub fn is_active(&self) -> bool {
        self.flow.with(|flow| flow.is_some())
    }

    fn apply(&self, event: &Event) -> Option<PolkitEvent> {
        batch(|| match event {
            Event::Available(value) => {
                self.available.set(*value);
                None
            }
            Event::Registered(value) => {
                self.registered.set(*value);
                None
            }
            Event::Flow(flow) => {
                if let Some(flow) = flow
                    && flow.supplementary_is_error
                    && !flow.supplementary_message.is_empty()
                {
                    self.error.set(Some(PolkitError::Message(flow.supplementary_message.clone())));
                }
                self.flow.set(flow.clone());
                None
            }
            Event::Outcome(Outcome::Started) => {
                self.error.set(None);
                Some(PolkitEvent::RequestStarted)
            }
            Event::Outcome(Outcome::Failed(message)) => {
                self.error.set(Some(match message {
                    Some(message) => PolkitError::Message(message.clone()),
                    None => PolkitError::Failed,
                }));
                Some(PolkitEvent::Failed)
            }
            Event::Outcome(Outcome::Succeeded) => {
                self.error.set(None);
                Some(PolkitEvent::Succeeded)
            }
            Event::Outcome(Outcome::Cancelled) => {
                self.error.set(None);
                Some(PolkitEvent::Cancelled)
            }
        })
    }
}

enum Command {
    Submit(String),
    Cancel,
    Select(usize),
}

pub struct Polkit {
    pub state: PolkitState,
    service: ServiceHandle<Command>,
}

impl Polkit {
    pub fn start<D: 'static>(
        handle: &LoopHandle<'static, D>,
        bus: BusKind,
        options: PolkitOptions,
        mut on_event: impl FnMut(PolkitEvent) + 'static,
    ) -> Result<Polkit, IoError> {
        let state = PolkitState::new();
        let target = state.clone();
        let service = spawn_worker(
            handle,
            "serp-polkit",
            move |event| {
                if let Some(event) = target.apply(&event) {
                    on_event(event);
                }
            },
            move |events, commands| run(bus, options, events, commands),
        )?;
        Ok(Polkit { state, service })
    }

    pub fn submit(&self, response: impl Into<String>) -> bool {
        self.service.send(Command::Submit(response.into()))
    }

    pub fn cancel(&self) -> bool {
        self.service.send(Command::Cancel)
    }

    pub fn select_identity(&self, index: usize) -> bool {
        self.service.send(Command::Select(index))
    }
}

#[derive(Debug, zbus::DBusError)]
#[zbus(prefix = "org.freedesktop.PolicyKit1.Error")]
enum AgentError {
    #[zbus(error)]
    ZBus(zbus::Error),
    Cancelled(String),
}

struct Request {
    action_id: String,
    message: String,
    icon_name: String,
    details: BTreeMap<String, String>,
    cookie: String,
    identities: Vec<PolkitIdentity>,
    done: async_channel::Sender<bool>,
}

enum Incoming {
    Begin(Request),
    Cancel(String),
}

struct Agent {
    incoming: async_channel::Sender<Incoming>,
    passwd: PathBuf,
    group: PathBuf,
}

fn raw_identity(kind: &str, details: &HashMap<String, OwnedVariant>) -> RawIdentity {
    let key = if kind == "unix-group" { "gid" } else { "uid" };
    let id = details.get(key).and_then(|value| match &**value {
        Value::U32(id) => Some(*id),
        _ => None,
    });
    RawIdentity { kind: kind.to_owned(), id }
}

#[interface(name = "org.freedesktop.PolicyKit1.AuthenticationAgent")]
impl Agent {
    async fn begin_authentication(
        &self,
        action_id: String,
        message: String,
        icon_name: String,
        details: HashMap<String, String>,
        cookie: String,
        identities: Vec<(String, HashMap<String, OwnedVariant>)>,
    ) -> Result<(), AgentError> {
        let raw: Vec<RawIdentity> = identities.iter().map(|(kind, details)| raw_identity(kind, details)).collect();
        let passwd = std::fs::read_to_string(&self.passwd).unwrap_or_default();
        let group = std::fs::read_to_string(&self.group).unwrap_or_default();
        let (done, finished) = async_channel::bounded(1);
        let request = Request {
            action_id,
            message,
            icon_name,
            details: details.into_iter().collect(),
            cookie,
            identities: resolve_identities(&raw, &passwd, &group),
            done,
        };
        if self.incoming.send(Incoming::Begin(request)).await.is_err() {
            return Err(AgentError::Cancelled("agent stopped".into()));
        }
        match finished.recv().await {
            Ok(true) => Ok(()),
            _ => Err(AgentError::Cancelled("authentication cancelled".into())),
        }
    }

    async fn cancel_authentication(&self, cookie: String) {
        let _ = self.incoming.send(Incoming::Cancel(cookie)).await;
    }
}

#[proxy(
    interface = "org.freedesktop.PolicyKit1.Authority",
    default_service = "org.freedesktop.PolicyKit1",
    default_path = "/org/freedesktop/PolicyKit1/Authority"
)]
trait Authority {
    fn register_authentication_agent(
        &self,
        subject: &(&str, HashMap<&str, Value<'_>>),
        locale: &str,
        object_path: &ObjectPath<'_>,
    ) -> zbus::Result<()>;

    fn unregister_authentication_agent(
        &self,
        subject: &(&str, HashMap<&str, Value<'_>>),
        object_path: &ObjectPath<'_>,
    ) -> zbus::Result<()>;
}

#[proxy(
    interface = "org.freedesktop.login1.Manager",
    default_service = "org.freedesktop.login1",
    default_path = "/org/freedesktop/login1"
)]
trait Login1 {
    fn get_session_by_pid(&self, pid: u32) -> zbus::Result<zbus::OwnedObjectPath>;
}

async fn session_id(connection: &Connection, options: &PolkitOptions) -> zbus::Result<String> {
    if let Some(id) = &options.session_id {
        return Ok(id.clone());
    }
    let manager = Login1Proxy::new(connection).await?;
    let path = manager.get_session_by_pid(std::process::id()).await?;
    let session =
        zbus::Proxy::new(connection, "org.freedesktop.login1", path, "org.freedesktop.login1.Session").await?;
    session.get_property::<String>("Id").await
}

async fn register(connection: &Connection, session: &str, options: &PolkitOptions) -> zbus::Result<()> {
    let authority = AuthorityProxy::new(connection).await?;
    let subject = ("unix-session", HashMap::from([("session-id", Value::from(session))]));
    let path = ObjectPath::try_from(options.agent_path.as_str())?;
    authority.register_authentication_agent(&subject, &options.locale, &path).await
}

async fn unregister(connection: &Connection, session: &str, options: &PolkitOptions) {
    let Ok(authority) = AuthorityProxy::new(connection).await else { return };
    let subject = ("unix-session", HashMap::from([("session-id", Value::from(session))]));
    let Ok(path) = ObjectPath::try_from(options.agent_path.as_str()) else { return };
    let _ = authority.unregister_authentication_agent(&subject, &path).await;
}

enum HelperMsg {
    Line(String),
    Closed,
}

type HelperSender = async_channel::Sender<(u64, HelperMsg)>;

struct Helper {
    writer: Box<dyn Write + Send>,
    stop: Option<Box<dyn FnOnce() + Send>>,
}

impl Helper {
    fn open(
        transport: &Transport,
        user: &str,
        cookie: &str,
        generation: u64,
        messages: HelperSender,
    ) -> std::io::Result<Helper> {
        match transport {
            Transport::Socket(path) => {
                let stream = UnixStream::connect(path)?;
                let reader = stream.try_clone()?;
                let mut writer = stream.try_clone()?;
                writeln!(writer, "{user}")?;
                writeln!(writer, "{cookie}")?;
                spawn_reader(Box::new(reader), generation, messages, None);
                let closer = stream;
                Ok(Helper {
                    writer: Box::new(writer),
                    stop: Some(Box::new(move || {
                        let _ = closer.shutdown(std::net::Shutdown::Both);
                    })),
                })
            }
            Transport::Spawn(path) => {
                let mut child = spawn_with_retry(path, user)?;
                let mut writer = child.stdin.take().ok_or(std::io::ErrorKind::BrokenPipe)?;
                let reader = child.stdout.take().ok_or(std::io::ErrorKind::BrokenPipe)?;
                writeln!(writer, "{cookie}")?;
                let pid = Arc::new(Mutex::new(rustix::process::Pid::from_raw(child.id() as i32)));
                spawn_reader(Box::new(reader), generation, messages, Some((child, pid.clone())));
                Ok(Helper {
                    writer: Box::new(writer),
                    stop: Some(Box::new(move || {
                        let pid = pid.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
                        if let Some(pid) = *pid {
                            let _ = rustix::process::kill_process(pid, rustix::process::Signal::TERM);
                        }
                    })),
                })
            }
        }
    }

    fn send_line(&mut self, line: &str) -> std::io::Result<()> {
        writeln!(self.writer, "{line}")?;
        self.writer.flush()
    }
}

impl Drop for Helper {
    fn drop(&mut self) {
        if let Some(stop) = self.stop.take() {
            stop();
        }
    }
}

fn spawn_with_retry(path: &Path, user: &str) -> std::io::Result<std::process::Child> {
    let mut attempt = 0;
    loop {
        let spawned =
            Process::new(path).arg(user).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::null()).spawn();
        match spawned {
            Err(e) if e.kind() == std::io::ErrorKind::ExecutableFileBusy && attempt < 10 => {
                attempt += 1;
                std::thread::sleep(Duration::from_millis(20));
            }
            other => return other,
        }
    }
}

type ChildSlot = (std::process::Child, Arc<Mutex<Option<rustix::process::Pid>>>);

fn spawn_reader(
    reader: Box<dyn std::io::Read + Send>,
    generation: u64,
    messages: HelperSender,
    child: Option<ChildSlot>,
) {
    std::thread::spawn(move || {
        let mut reader = BufReader::new(reader);
        let mut buffer = Vec::new();
        loop {
            buffer.clear();
            match reader.read_until(b'\n', &mut buffer) {
                Ok(0) | Err(_) => break,
                Ok(_) => {
                    let line = String::from_utf8_lossy(&buffer).trim_end_matches(['\r', '\n']).to_owned();
                    if messages.send_blocking((generation, HelperMsg::Line(line))).is_err() {
                        break;
                    }
                }
            }
        }
        if let Some((mut child, pid)) = child {
            *pid.lock().unwrap_or_else(|poisoned| poisoned.into_inner()) = None;
            let _ = child.wait();
        }
        let _ = messages.send_blocking((generation, HelperMsg::Closed));
    });
}

struct Active {
    done: async_channel::Sender<bool>,
    flow: PolkitFlow,
    helper: Option<Helper>,
    generation: u64,
}

struct Session<'a> {
    transport: Option<Transport>,
    events: &'a EventSink<Event>,
    messages: HelperSender,
    queue: VecDeque<Request>,
    active: Option<Active>,
    next_generation: u64,
}

impl Session<'_> {
    fn enqueue(&mut self, request: Request) {
        self.queue.push_back(request);
        self.advance();
    }

    fn advance(&mut self) {
        while self.active.is_none() {
            let Some(request) = self.queue.pop_front() else { break };
            self.start(request);
        }
    }

    fn start(&mut self, request: Request) {
        if request.identities.is_empty() {
            tracing::warn!("polkit request {} has no usable identity", request.action_id);
            let _ = request.done.try_send(false);
            return;
        }
        let selected = default_identity(&request.identities, current_uid(&request.identities));
        let flow = PolkitFlow {
            cookie: request.cookie,
            action_id: request.action_id,
            message: request.message,
            icon_name: request.icon_name,
            details: request.details,
            identities: request.identities,
            selected,
            response_required: false,
            input_prompt: String::new(),
            echo: false,
            supplementary_message: String::new(),
            supplementary_is_error: false,
        };
        self.events.send(Event::Flow(Some(flow.clone())));
        self.events.send(Event::Outcome(Outcome::Started));
        let mut active = Active { done: request.done, flow, helper: None, generation: 0 };
        match self.begin_session(&mut active) {
            Ok(()) => self.active = Some(active),
            Err(e) => {
                tracing::warn!("polkit helper could not start: {e}");
                let _ = active.done.try_send(false);
                self.events.send(Event::Flow(None));
                self.events.send(Event::Outcome(Outcome::Cancelled));
            }
        }
    }

    fn begin_session(&mut self, active: &mut Active) -> std::io::Result<()> {
        active.helper = None;
        let transport = self.transport.clone().ok_or(std::io::ErrorKind::NotFound)?;
        let user = active.flow.identity().map(|identity| identity.name.clone()).ok_or(std::io::ErrorKind::NotFound)?;
        self.next_generation += 1;
        active.generation = self.next_generation;
        active.helper =
            Some(Helper::open(&transport, &user, &active.flow.cookie, active.generation, self.messages.clone())?);
        active.flow.reset_session();
        Ok(())
    }

    fn finish(&mut self, success: bool, outcome: Outcome) {
        if let Some(active) = self.active.take() {
            drop(active.helper);
            let _ = active.done.try_send(success);
            self.events.send(Event::Flow(None));
            self.events.send(Event::Outcome(outcome));
        }
        self.advance();
    }

    fn cancel_active(&mut self) {
        self.finish(false, Outcome::Cancelled);
    }

    fn cancel_cookie(&mut self, cookie: &str) {
        if self.active.as_ref().is_some_and(|active| active.flow.cookie == cookie) {
            self.cancel_active();
        } else if let Some(request) = self
            .queue
            .iter()
            .position(|request| request.cookie == cookie)
            .and_then(|position| self.queue.remove(position))
        {
            let _ = request.done.try_send(false);
        }
    }

    fn submit(&mut self, response: &str) {
        let Some(active) = self.active.as_mut() else { return };
        if !active.flow.response_required {
            return;
        }
        let Some(helper) = active.helper.as_mut() else { return };
        if helper.send_line(response).is_err() {
            return;
        }
        active.flow.response_required = false;
        self.events.send(Event::Flow(Some(active.flow.clone())));
    }

    fn select(&mut self, index: usize) {
        let Some(mut active) = self.active.take() else { return };
        if index < active.flow.identities.len() && index != active.flow.selected {
            active.flow.selected = index;
            match self.begin_session(&mut active) {
                Ok(()) => self.events.send(Event::Flow(Some(active.flow.clone()))),
                Err(e) => {
                    tracing::warn!("polkit helper could not restart: {e}");
                    self.active = Some(active);
                    self.cancel_active();
                    return;
                }
            };
        }
        self.active = Some(active);
    }

    fn helper_message(&mut self, generation: u64, message: HelperMsg) {
        let Some(active) = self.active.as_mut() else { return };
        if active.generation != generation {
            return;
        }
        match message {
            HelperMsg::Line(line) => match active.flow.apply_line(&line) {
                LineResult::Prompt | LineResult::Message => {
                    self.events.send(Event::Flow(Some(active.flow.clone())));
                }
                LineResult::Success => self.finish(true, Outcome::Succeeded),
                LineResult::Failure => {
                    let message = Some(active.flow.supplementary_message.clone()).filter(|m| !m.is_empty());
                    self.events.send(Event::Outcome(Outcome::Failed(message)));
                    let mut active = self.active.take().expect("active request");
                    match self.begin_session(&mut active) {
                        Ok(()) => {
                            self.events.send(Event::Flow(Some(active.flow.clone())));
                            self.active = Some(active);
                        }
                        Err(e) => {
                            tracing::warn!("polkit helper could not restart: {e}");
                            self.active = Some(active);
                            self.cancel_active();
                        }
                    }
                }
                LineResult::Ignored => {}
            },
            HelperMsg::Closed => {
                tracing::warn!("polkit helper exited before finishing");
                self.cancel_active();
            }
        }
    }

    fn stop(&mut self) {
        self.cancel_active();
        while let Some(request) = self.queue.pop_front() {
            let _ = request.done.try_send(false);
        }
    }
}

fn current_uid(identities: &[PolkitIdentity]) -> u32 {
    let uid = rustix::process::getuid().as_raw();
    identities.iter().find(|identity| identity.uid == uid).map_or(uid, |identity| identity.uid)
}

enum Input {
    Incoming(Incoming),
    Command(Command),
    Helper(u64, HelperMsg),
    Owner(bool),
    Register,
    Stop,
}

async fn run(
    bus: BusKind,
    options: PolkitOptions,
    events: EventSink<Event>,
    commands: async_channel::Receiver<Command>,
) {
    let mut commands = commands;
    loop {
        match bus.connect().await {
            Ok(connection) => {
                if serve(&connection, &options, &events, &mut commands).await {
                    return;
                }
            }
            Err(e) => tracing::info!("polkit agent could not connect: {e}"),
        }
        events.send(Event::Available(false));
        events.send(Event::Registered(false));
        let retry = async {
            async_io::Timer::after(options.retry_delay).await;
            false
        };
        let closed = async {
            while commands.recv().await.is_ok() {}
            true
        };
        if closed.or(retry).await {
            return;
        }
    }
}

async fn serve(
    connection: &Connection,
    options: &PolkitOptions,
    events: &EventSink<Event>,
    commands: &mut async_channel::Receiver<Command>,
) -> bool {
    let (incoming_tx, incoming_rx) = async_channel::unbounded();
    let agent = Agent { incoming: incoming_tx, passwd: options.passwd.clone(), group: options.group.clone() };
    if let Err(e) = connection.object_server().at(options.agent_path.as_str(), agent).await {
        tracing::warn!("polkit agent object could not be served: {e}");
        return false;
    }
    let session = match session_id(connection, options).await {
        Ok(session) => session,
        Err(e) => {
            tracing::info!("polkit agent has no session: {e}");
            return false;
        }
    };
    let Ok(dbus) = DBusProxy::new(connection).await else { return false };
    let Ok(mut owners) = dbus.receive_name_owner_changed().await else { return false };
    events.send(Event::Available(true));
    let (message_tx, message_rx) = async_channel::unbounded();
    let mut state = Session {
        transport: options.transport.clone(),
        events,
        messages: message_tx,
        queue: VecDeque::new(),
        active: None,
        next_generation: 0,
    };
    let mut register_at = Some(Instant::now());
    let mut registered = false;
    loop {
        let input = async { incoming_rx.recv().await.ok().map(Input::Incoming) }
            .or(async { Some(commands.recv().await.map_or(Input::Stop, Input::Command)) })
            .or(async { message_rx.recv().await.ok().map(|(generation, message)| Input::Helper(generation, message)) })
            .or(async {
                loop {
                    let signal = owners.next().await?;
                    let args = signal.args().ok()?;
                    if args.name().as_str() == AUTHORITY_NAME {
                        return Some(Input::Owner(args.new_owner().is_some()));
                    }
                }
            })
            .or(async {
                match register_at {
                    Some(at) => {
                        async_io::Timer::at(at).await;
                    }
                    None => future::pending::<()>().await,
                }
                Some(Input::Register)
            })
            .await;
        match input {
            Some(Input::Incoming(Incoming::Begin(request))) => state.enqueue(request),
            Some(Input::Incoming(Incoming::Cancel(cookie))) => state.cancel_cookie(&cookie),
            Some(Input::Command(Command::Submit(response))) => state.submit(&response),
            Some(Input::Command(Command::Cancel)) => state.cancel_active(),
            Some(Input::Command(Command::Select(index))) => state.select(index),
            Some(Input::Helper(generation, message)) => state.helper_message(generation, message),
            Some(Input::Owner(true)) => {
                registered = false;
                events.send(Event::Registered(false));
                register_at = Some(Instant::now());
            }
            Some(Input::Owner(false)) => {
                registered = false;
                events.send(Event::Registered(false));
                register_at = None;
            }
            Some(Input::Register) => match register(connection, &session, options).await {
                Ok(()) => {
                    registered = true;
                    register_at = None;
                    events.send(Event::Registered(true));
                }
                Err(e) => {
                    tracing::info!("polkit agent registration failed: {e}");
                    register_at = Some(Instant::now() + options.retry_delay);
                }
            },
            Some(Input::Stop) => {
                state.stop();
                if registered {
                    unregister(connection, &session, options).await;
                }
                return true;
            }
            None => return false,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::io::Read;
    use std::os::unix::fs::PermissionsExt;
    use std::os::unix::net::UnixListener;
    use std::rc::Rc;
    use std::sync::mpsc;

    use calloop::EventLoop;

    use super::*;
    use crate::platform::dbus::testbus::TestBus;

    const PASSWD: &str = "root:x:0:0:root:/root:/bin/sh\nalice:x:1000:100:Alice:/home/alice:/bin/zsh\nbob:x:1001:101::/home/bob:/bin/sh\ncarol:x:1002:100::/home/carol:/bin/sh\n";
    const GROUP: &str = "root:x:0:\nusers:x:100:\nwheel:x:10:bob,carol\nbobs:x:101:\n";

    fn flow() -> PolkitFlow {
        PolkitFlow {
            cookie: "c".into(),
            action_id: "org.example.run".into(),
            message: "Run it".into(),
            icon_name: String::new(),
            details: BTreeMap::new(),
            identities: vec![PolkitIdentity { uid: 1000, name: "alice".into() }],
            selected: 0,
            response_required: false,
            input_prompt: String::new(),
            echo: false,
            supplementary_message: String::new(),
            supplementary_is_error: false,
        }
    }

    #[test]
    fn passwd_and_group_files_parse() {
        assert_eq!(parse_passwd(PASSWD)[1], ("alice".to_owned(), 1000, 100));
        assert_eq!(parse_passwd("broken\n:x:1:1\n").len(), 0);
        assert_eq!(parse_group(GROUP)[2], (10, vec!["bob".to_owned(), "carol".to_owned()]));
    }

    #[test]
    fn identities_resolve_users_and_expand_groups() {
        let raw = |kind: &str, id| RawIdentity { kind: kind.to_owned(), id: Some(id) };
        let users = resolve_identities(&[raw("unix-user", 1001), raw("unix-user", 4242)], PASSWD, GROUP);
        assert_eq!(users, vec![PolkitIdentity { uid: 1001, name: "bob".into() }]);
        let wheel = resolve_identities(&[raw("unix-group", 10)], PASSWD, GROUP);
        assert_eq!(wheel.iter().map(|i| i.name.as_str()).collect::<Vec<_>>(), ["bob", "carol"]);
        let primary = resolve_identities(&[raw("unix-group", 100), raw("unix-user", 1000)], PASSWD, GROUP);
        assert_eq!(primary.iter().map(|i| i.name.as_str()).collect::<Vec<_>>(), ["alice", "carol"]);
        assert!(resolve_identities(&[raw("unix-netgroup", 1)], PASSWD, GROUP).is_empty());
        assert_eq!(default_identity(&primary, 1002), 1);
        assert_eq!(default_identity(&primary, 7), 0);
    }

    #[test]
    fn helper_lines_drive_the_flow() {
        let mut flow = flow();
        assert_eq!(flow.apply_line("PAM_PROMPT_ECHO_OFF Password: "), LineResult::Prompt);
        assert!(flow.response_required && !flow.echo);
        assert_eq!(flow.input_prompt, "Password: ");
        assert_eq!(flow.apply_line("PAM_PROMPT_ECHO_ON Code:"), LineResult::Prompt);
        assert!(flow.echo);
        assert_eq!(flow.apply_line("PAM_ERROR_MSG Wrong fingerprint"), LineResult::Message);
        assert!(flow.supplementary_is_error);
        assert_eq!(flow.apply_line("PAM_TEXT_INFO Touch the sensor"), LineResult::Message);
        assert!(!flow.supplementary_is_error);
        assert_eq!(flow.supplementary_message, "Touch the sensor");
        assert_eq!(flow.apply_line("SUCCESS"), LineResult::Success);
        assert_eq!(flow.apply_line("FAILURE\n"), LineResult::Failure);
        assert_eq!(flow.apply_line("noise"), LineResult::Ignored);
        flow.reset_session();
        assert!(!flow.response_required && flow.input_prompt.is_empty() && flow.supplementary_message.is_empty());
    }

    #[test]
    fn transport_prefers_the_socket_then_known_directories_then_path() {
        let socket = PathBuf::from(HELPER_SOCKET);
        assert_eq!(Transport::discover_with(&[], |p| p == socket), Some(Transport::Socket(socket.clone())));
        let wrapper = Path::new(HELPER_DIRS[0]).join(HELPER_NAME);
        assert_eq!(Transport::discover_with(&[], |p| p == wrapper), Some(Transport::Spawn(wrapper.clone())));
        let custom = PathBuf::from("custom/bin");
        let found = custom.join(HELPER_NAME);
        assert_eq!(Transport::discover_with(&[custom], |p| p == found), Some(Transport::Spawn(found)));
        assert_eq!(Transport::discover_with(&[], |_| false), None);
    }

    #[test]
    fn state_tracks_flow_and_error_like_polkit_service_qml() {
        let state = PolkitState::new();
        assert!(!state.is_active());
        assert_eq!(state.apply(&Event::Flow(Some(flow()))), None);
        assert_eq!(state.apply(&Event::Outcome(Outcome::Started)), Some(PolkitEvent::RequestStarted));
        assert!(state.is_active());
        let mut with_error = flow();
        with_error.supplementary_message = "Try again".into();
        with_error.supplementary_is_error = true;
        state.apply(&Event::Flow(Some(with_error)));
        assert_eq!(state.error.get_untracked(), Some(PolkitError::Message("Try again".into())));
        assert_eq!(state.apply(&Event::Outcome(Outcome::Failed(None))), Some(PolkitEvent::Failed));
        assert_eq!(state.error.get_untracked(), Some(PolkitError::Failed));
        let mut info = flow();
        info.supplementary_message = "Touch".into();
        state.apply(&Event::Flow(Some(info)));
        assert_eq!(state.error.get_untracked(), Some(PolkitError::Failed));
        assert_eq!(state.apply(&Event::Outcome(Outcome::Succeeded)), Some(PolkitEvent::Succeeded));
        assert_eq!(state.error.get_untracked(), None);
        state.apply(&Event::Flow(None));
        assert!(!state.is_active());
        state.apply(&Event::Outcome(Outcome::Failed(Some("Locked".into()))));
        assert_eq!(state.apply(&Event::Outcome(Outcome::Cancelled)), Some(PolkitEvent::Cancelled));
        assert_eq!(state.error.get_untracked(), None);
    }

    fn pump_until(event_loop: &mut EventLoop<'static, ()>, done: impl Fn() -> bool) -> bool {
        let deadline = Instant::now() + Duration::from_secs(15);
        while !done() && Instant::now() < deadline {
            event_loop.dispatch(Duration::from_millis(10), &mut ()).unwrap();
        }
        done()
    }

    struct FakeAuthority {
        registered: mpsc::Sender<(String, String, String, String)>,
        unregistered: mpsc::Sender<()>,
    }

    #[interface(name = "org.freedesktop.PolicyKit1.Authority")]
    impl FakeAuthority {
        async fn register_authentication_agent(
            &self,
            subject: (String, HashMap<String, OwnedVariant>),
            locale: String,
            object_path: ObjectPath<'_>,
            #[zbus(header)] header: zbus::message::Header<'_>,
        ) {
            let session = match subject.1.get("session-id").map(|value| &**value) {
                Some(Value::Str(id)) => id.to_string(),
                _ => String::new(),
            };
            let sender = header.sender().map(|s| s.to_string()).unwrap_or_default();
            let _ = self.registered.send((sender, object_path.to_string(), session, locale));
        }

        async fn unregister_authentication_agent(
            &self,
            _subject: (String, HashMap<String, OwnedVariant>),
            _path: ObjectPath<'_>,
        ) {
            let _ = self.unregistered.send(());
        }
    }

    fn write_helper(dir: &Path, log: &Path) -> PathBuf {
        let path = dir.join("helper");
        let script = format!(
            "#!/bin/sh\nread cookie\necho \"$1 $cookie\" >> {log}\nprintf 'PAM_PROMPT_ECHO_OFF Password: \\n'\nread answer\nif [ \"$answer\" = secret ]; then\n  printf 'SUCCESS\\n'\nelse\n  printf 'PAM_ERROR_MSG Sorry, try again.\\nFAILURE\\n'\nfi\n",
            log = log.display()
        );
        std::fs::write(&path, script).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        path
    }

    struct Harness {
        event_loop: EventLoop<'static, ()>,
        polkit: Polkit,
        events: Rc<RefCell<Vec<PolkitEvent>>>,
        address: String,
        agent: (String, String),
        _bus: TestBus,
        _dir: tempfile::TempDir,
        log: PathBuf,
        unregistered: mpsc::Receiver<()>,
    }

    fn harness(transport: impl FnOnce(&Path, &Path) -> Transport) -> Harness {
        let bus = TestBus::start();
        let dir = tempfile::tempdir().unwrap();
        let log = dir.path().join("helper.log");
        let passwd = dir.path().join("passwd");
        let group = dir.path().join("group");
        let uid = rustix::process::getuid().as_raw();
        std::fs::write(&passwd, format!("tester:x:{uid}:{uid}::/home/tester:/bin/sh\nother:x:4242:4242::/h:/bin/sh\n"))
            .unwrap();
        std::fs::write(&group, "").unwrap();
        let transport = transport(dir.path(), &log);
        let (registered_tx, registered_rx) = mpsc::channel();
        let (unregistered_tx, unregistered_rx) = mpsc::channel();
        let address = bus.address.clone();
        let (ready_tx, ready_rx) = mpsc::channel();
        std::thread::spawn(move || {
            async_io::block_on(async move {
                let connection = zbus::connection::Builder::address(address.as_str())
                    .name(AUTHORITY_NAME)
                    .serve_at(
                        "/org/freedesktop/PolicyKit1/Authority",
                        FakeAuthority { registered: registered_tx, unregistered: unregistered_tx },
                    )
                    .build()
                    .await
                    .unwrap();
                ready_tx.send(()).unwrap();
                std::future::pending::<()>().await;
                drop(connection);
            });
        });
        ready_rx.recv_timeout(Duration::from_secs(10)).unwrap();
        let event_loop: EventLoop<'static, ()> = EventLoop::try_new().unwrap();
        let events = Rc::new(RefCell::new(Vec::new()));
        let sink = events.clone();
        let options = PolkitOptions {
            transport: Some(transport),
            session_id: Some("7".into()),
            agent_path: agent_path("demo"),
            locale: "en_GB.UTF-8".into(),
            retry_delay: Duration::from_millis(200),
            passwd,
            group,
            uid,
        };
        let polkit =
            Polkit::start(&event_loop.handle(), bus.kind(), options, move |event| sink.borrow_mut().push(event))
                .unwrap();
        let (sender, path, session, locale) = registered_rx.recv_timeout(Duration::from_secs(10)).unwrap();
        assert_eq!(
            (path.as_str(), session.as_str(), locale.as_str()),
            (agent_path("demo").as_str(), "7", "en_GB.UTF-8")
        );
        let address = bus.address.clone();
        Harness {
            event_loop,
            polkit,
            events,
            address,
            agent: (sender, path),
            _bus: bus,
            _dir: dir,
            log,
            unregistered: unregistered_rx,
        }
    }

    fn begin(harness: &Harness, cookie: &str) -> mpsc::Receiver<Result<(), String>> {
        let (tx, rx) = mpsc::channel();
        let address = harness.address.clone();
        let (sender, path) = harness.agent.clone();
        let cookie = cookie.to_owned();
        std::thread::spawn(move || {
            async_io::block_on(async move {
                let connection = zbus::connection::Builder::address(address.as_str()).build().await.unwrap();
                let details: HashMap<&str, &str> = HashMap::from([("polkit.message", "x")]);
                let identities: Vec<(&str, HashMap<&str, Value>)> = vec![
                    ("unix-user", HashMap::from([("uid", Value::U32(4242))])),
                    ("unix-user", HashMap::from([("uid", Value::U32(rustix::process::getuid().as_raw()))])),
                ];
                let reply = connection
                    .call_method(
                        Some(sender.as_str()),
                        path.as_str(),
                        Some("org.freedesktop.PolicyKit1.AuthenticationAgent"),
                        "BeginAuthentication",
                        &(
                            "org.example.run",
                            "Run the example",
                            "dialog-password",
                            details,
                            cookie.as_str(),
                            identities,
                        ),
                    )
                    .await;
                tx.send(reply.map(|_| ()).map_err(|e| e.to_string())).unwrap();
            });
        });
        rx
    }

    fn cancel_from_polkitd(harness: &Harness, cookie: &str) {
        let address = harness.address.clone();
        let (sender, path) = harness.agent.clone();
        let cookie = cookie.to_owned();
        std::thread::spawn(move || {
            async_io::block_on(async move {
                let connection = zbus::connection::Builder::address(address.as_str()).build().await.unwrap();
                connection
                    .call_method(
                        Some(sender.as_str()),
                        path.as_str(),
                        Some("org.freedesktop.PolicyKit1.AuthenticationAgent"),
                        "CancelAuthentication",
                        &(cookie.as_str(),),
                    )
                    .await
                    .unwrap();
            });
        });
    }

    fn spawn_transport(dir: &Path, log: &Path) -> Transport {
        Transport::Spawn(write_helper(dir, log))
    }

    #[test]
    fn agent_registers_and_authenticates_through_the_helper_protocol() {
        let mut h = harness(spawn_transport);
        let state = h.polkit.state.clone();
        assert!(pump_until(&mut h.event_loop, || state.registered.get_untracked()));
        let result = begin(&h, "cookie-1");
        assert!(pump_until(&mut h.event_loop, || state.flow.with(|f| f.as_ref().is_some_and(|f| f.response_required))));
        let flow = state.flow.get_untracked().unwrap();
        assert_eq!(flow.action_id, "org.example.run");
        assert_eq!(flow.message, "Run the example");
        assert_eq!(flow.icon_name, "dialog-password");
        assert_eq!(flow.input_prompt, "Password: ");
        assert_eq!(flow.identities.len(), 2);
        assert_eq!(flow.selected, 1, "the current user is preselected");
        assert_eq!(flow.identity().unwrap().name, "tester");
        assert!(state.is_active());

        assert!(h.polkit.submit("wrong"));
        assert!(pump_until(&mut h.event_loop, || h.events.borrow().contains(&PolkitEvent::Failed)));
        assert!(pump_until(&mut h.event_loop, || state.flow.with(|f| f.as_ref().is_some_and(|f| f.response_required))));
        assert_eq!(state.error.get_untracked(), Some(PolkitError::Message("Sorry, try again.".into())));
        assert!(result.try_recv().is_err(), "polkitd is still waiting after a wrong password");

        assert!(h.polkit.submit("secret"));
        assert!(pump_until(&mut h.event_loop, || h.events.borrow().contains(&PolkitEvent::Succeeded)));
        assert_eq!(result.recv_timeout(Duration::from_secs(10)).unwrap(), Ok(()));
        assert!(!state.is_active());
        assert_eq!(state.error.get_untracked(), None);
        let log = std::fs::read_to_string(&h.log).unwrap();
        assert_eq!(log.lines().collect::<Vec<_>>(), ["tester cookie-1", "tester cookie-1"]);
        assert_eq!(h.events.borrow()[0], PolkitEvent::RequestStarted);

        drop(h.polkit);
        assert!(h.unregistered.recv_timeout(Duration::from_secs(10)).is_ok());
    }

    #[test]
    fn cancelling_returns_the_polkit_cancelled_error() {
        let mut h = harness(spawn_transport);
        let state = h.polkit.state.clone();
        let result = begin(&h, "cookie-2");
        assert!(pump_until(&mut h.event_loop, || state.flow.with(|f| f.as_ref().is_some_and(|f| f.response_required))));
        assert!(h.polkit.cancel());
        assert!(pump_until(&mut h.event_loop, || h.events.borrow().contains(&PolkitEvent::Cancelled)));
        let error = result.recv_timeout(Duration::from_secs(10)).unwrap().unwrap_err();
        assert!(error.contains("org.freedesktop.PolicyKit1.Error.Cancelled"), "{error}");
        assert!(!state.is_active());
    }

    #[test]
    fn polkitd_cancel_ends_the_active_request_and_queued_requests_follow() {
        let mut h = harness(spawn_transport);
        let state = h.polkit.state.clone();
        let first = begin(&h, "first");
        assert!(pump_until(&mut h.event_loop, || state.flow.with(|f| f.as_ref().is_some_and(|f| f.cookie == "first"))));
        let second = begin(&h, "second");
        std::thread::sleep(Duration::from_millis(300));
        h.event_loop.dispatch(Duration::from_millis(10), &mut ()).unwrap();
        assert_eq!(state.flow.get_untracked().unwrap().cookie, "first", "requests are handled one at a time");
        cancel_from_polkitd(&h, "first");
        assert!(first.recv_timeout(Duration::from_secs(10)).unwrap().is_err());
        assert!(pump_until(&mut h.event_loop, || state
            .flow
            .with(|f| f.as_ref().is_some_and(|f| f.cookie == "second"))));
        assert!(pump_until(&mut h.event_loop, || state.flow.with(|f| f.as_ref().is_some_and(|f| f.response_required))));
        h.polkit.submit("secret");
        assert_eq!(second.recv_timeout(Duration::from_secs(10)).unwrap(), Ok(()));
    }

    #[test]
    fn socket_transport_sends_user_and_cookie_then_speaks_the_same_protocol() {
        let socket_dir = tempfile::tempdir().unwrap();
        let socket = socket_dir.path().join("helper.socket");
        let listener = UnixListener::bind(&socket).unwrap();
        let (seen_tx, seen_rx) = mpsc::channel();
        std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            let mut user = String::new();
            let mut cookie = String::new();
            reader.read_line(&mut user).unwrap();
            reader.read_line(&mut cookie).unwrap();
            stream.write_all(b"PAM_PROMPT_ECHO_OFF Password:\n").unwrap();
            let mut answer = String::new();
            reader.read_line(&mut answer).unwrap();
            stream.write_all(b"SUCCESS\n").unwrap();
            seen_tx.send((user, cookie, answer)).unwrap();
            let mut rest = Vec::new();
            let _ = stream.read_to_end(&mut rest);
        });
        let mut h = harness(move |_, _| Transport::Socket(socket));
        let state = h.polkit.state.clone();
        let result = begin(&h, "socket-cookie");
        assert!(pump_until(&mut h.event_loop, || state.flow.with(|f| f.as_ref().is_some_and(|f| f.response_required))));
        h.polkit.submit("hunter2");
        assert_eq!(result.recv_timeout(Duration::from_secs(10)).unwrap(), Ok(()));
        let (user, cookie, answer) = seen_rx.recv_timeout(Duration::from_secs(10)).unwrap();
        assert_eq!((user.trim(), cookie.trim(), answer.trim()), ("tester", "socket-cookie", "hunter2"));
    }

    #[test]
    fn selecting_another_identity_restarts_the_helper_for_that_user() {
        let mut h = harness(spawn_transport);
        let state = h.polkit.state.clone();
        let result = begin(&h, "pick");
        assert!(pump_until(&mut h.event_loop, || state.flow.with(|f| f.as_ref().is_some_and(|f| f.response_required))));
        assert!(h.polkit.select_identity(0));
        assert!(pump_until(&mut h.event_loop, || state.flow.with(|f| f.as_ref().is_some_and(|f| f.selected == 0))));
        assert!(pump_until(&mut h.event_loop, || state.flow.with(|f| f.as_ref().is_some_and(|f| f.response_required))));
        h.polkit.submit("secret");
        assert_eq!(result.recv_timeout(Duration::from_secs(10)).unwrap(), Ok(()));
        let log = std::fs::read_to_string(&h.log).unwrap();
        assert_eq!(log.lines().collect::<Vec<_>>(), ["tester pick", "other pick"]);
    }

    #[test]
    fn a_missing_helper_cancels_the_request() {
        let mut h = harness(|dir, _| Transport::Spawn(dir.join("does-not-exist")));
        let state = h.polkit.state.clone();
        let result = begin(&h, "lost");
        assert!(pump_until(&mut h.event_loop, || h.events.borrow().contains(&PolkitEvent::Cancelled)));
        assert!(result.recv_timeout(Duration::from_secs(10)).unwrap().is_err());
        assert!(!state.is_active());
    }

    #[test]
    fn a_helper_that_dies_silently_cancels_the_request() {
        let mut h = harness(|dir, _| {
            let path = dir.join("dying");
            std::fs::write(&path, "#!/bin/sh\nexit 1\n").unwrap();
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
            Transport::Spawn(path)
        });
        let result = begin(&h, "dying");
        assert!(pump_until(&mut h.event_loop, || h.events.borrow().contains(&PolkitEvent::Cancelled)));
        assert!(result.recv_timeout(Duration::from_secs(10)).unwrap().is_err());
    }

    #[test]
    fn live_helper_socket_speaks_the_prompt_protocol() {
        if std::env::var_os("SERP_LIVE_POLKIT").is_none() {
            return;
        }
        let Some(Transport::Socket(path)) = Transport::discover().filter(|t| matches!(t, Transport::Socket(_))) else {
            return;
        };
        let user = std::env::var("USER").unwrap();
        let mut stream = UnixStream::connect(path).unwrap();
        writeln!(stream, "{user}").unwrap();
        writeln!(stream, "serp-rust-test-cookie").unwrap();
        stream.set_read_timeout(Some(Duration::from_secs(10))).unwrap();
        let mut line = String::new();
        BufReader::new(stream.try_clone().unwrap()).read_line(&mut line).unwrap();
        println!("helper said: {line:?}");
        assert!(line.starts_with("PAM_PROMPT_ECHO_OFF") || line.starts_with("PAM_"), "{line:?}");
        let _ = stream.shutdown(std::net::Shutdown::Both);
    }

    #[test]
    fn live_system_bus_registration() {
        if std::env::var_os("SERP_LIVE_POLKIT").is_none() {
            return;
        }
        let mut event_loop: EventLoop<'static, ()> = EventLoop::try_new().unwrap();
        let options = PolkitOptions {
            agent_path: "/org/demo/PolicyKit1/TestAgent".into(),
            retry_delay: Duration::from_secs(60),
            ..PolkitOptions::for_app("demo")
        };
        let polkit = Polkit::start(&event_loop.handle(), BusKind::System, options, |_| {}).unwrap();
        let state = polkit.state.clone();
        assert!(pump_until(&mut event_loop, || state.available.get_untracked()));
        let deadline = Instant::now() + Duration::from_secs(3);
        while Instant::now() < deadline && !state.registered.get_untracked() {
            event_loop.dispatch(Duration::from_millis(10), &mut ()).unwrap();
        }
        println!("registered with the real polkitd: {}", state.registered.get_untracked());
    }
}
