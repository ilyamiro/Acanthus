use std::collections::HashMap;
use std::path::PathBuf;

use acanthus_io::IoError;
use calloop::LoopHandle;
use futures_lite::{FutureExt, StreamExt};
use zbus::Variant;
use zbus::{OwnedObjectPath, OwnedVariant};

use crate::platform::bridge::{EventSink, ServiceHandle, spawn_worker};
use crate::platform::dbus::BusKind;

const SERVICE: &str = "org.freedesktop.portal.Desktop";
const PATH: &str = "/org/freedesktop/portal/desktop";
const INTERFACE: &str = "org.freedesktop.portal.FileChooser";
const SKI_BACKEND: &str = "org.freedesktop.impl.portal.desktop.ski";
const REQUEST: &str = "org.freedesktop.portal.Request";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PickMode {
    OpenFile,
    OpenFiles,
    Folder,
    Save,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Pattern {
    Glob(String),
    Mime(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Filter {
    pub name: String,
    pub patterns: Vec<Pattern>,
}

impl Filter {
    pub fn globs(name: &str, globs: &[&str]) -> Filter {
        Filter { name: name.to_owned(), patterns: globs.iter().map(|g| Pattern::Glob((*g).to_owned())).collect() }
    }

    fn wire(&self) -> (String, Vec<(u32, String)>) {
        let patterns = self
            .patterns
            .iter()
            .map(|pattern| match pattern {
                Pattern::Glob(text) => (0, text.clone()),
                Pattern::Mime(text) => (1, text.clone()),
            })
            .collect();
        (self.name.clone(), patterns)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PickSpec {
    pub mode: PickMode,
    pub title: String,
    pub filters: Vec<Filter>,
    pub folder: Option<PathBuf>,
    pub name: Option<String>,
}

impl PickSpec {
    pub fn new(mode: PickMode, title: &str) -> PickSpec {
        PickSpec { mode, title: title.to_owned(), filters: Vec::new(), folder: None, name: None }
    }

    pub fn filter(mut self, filter: Filter) -> PickSpec {
        self.filters.push(filter);
        self
    }

    pub fn folder(mut self, folder: PathBuf) -> PickSpec {
        self.folder = Some(folder);
        self
    }

    pub fn name(mut self, name: &str) -> PickSpec {
        self.name = Some(name.to_owned());
        self
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    Picked(Vec<PathBuf>),
    Cancelled,
    Unavailable,
}

pub fn request_path(unique_name: &str, token: &str) -> String {
    let sender = unique_name.trim_start_matches(':').replace('.', "_");
    format!("/org/freedesktop/portal/desktop/request/{sender}/{token}")
}

pub fn percent_decode(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        let hex = |byte: u8| (byte as char).to_digit(16);
        match (bytes[index], bytes.get(index + 1).copied().and_then(hex), bytes.get(index + 2).copied().and_then(hex)) {
            (b'%', Some(high), Some(low)) => {
                out.push((high * 16 + low) as u8);
                index += 3;
            }
            (byte, _, _) => {
                out.push(byte);
                index += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

pub fn uri_to_path(uri: &str) -> Option<PathBuf> {
    let rest = uri.strip_prefix("file://")?;
    let path = rest.find('/').map_or("", |slash| &rest[slash..]);
    if path.is_empty() { None } else { Some(PathBuf::from(percent_decode(path))) }
}

pub fn path_to_uri(path: &std::path::Path) -> String {
    use std::os::unix::ffi::OsStrExt;
    let mut out = String::from("file://");
    for &byte in path.as_os_str().as_bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' | b'/' => out.push(byte as char),
            other => out.push_str(&format!("%{other:02X}")),
        }
    }
    out
}

pub fn outcome(code: u32, uris: &[String]) -> Outcome {
    match code {
        0 => {
            let paths: Vec<PathBuf> = uris.iter().filter_map(|uri| uri_to_path(uri)).collect();
            if paths.is_empty() { Outcome::Cancelled } else { Outcome::Picked(paths) }
        }
        _ => Outcome::Cancelled,
    }
}

fn byte_string(path: &std::path::Path) -> Vec<u8> {
    use std::os::unix::ffi::OsStrExt;
    let mut bytes = path.as_os_str().as_bytes().to_vec();
    bytes.push(0);
    bytes
}

pub fn options<'a>(spec: &'a PickSpec, token: &'a str) -> HashMap<&'static str, Variant<'a>> {
    let mut options: HashMap<&'static str, Variant<'a>> = HashMap::new();
    options.insert("handle_token", Variant::from(token));
    options.insert("modal", Variant::from(true));
    match spec.mode {
        PickMode::OpenFiles => {
            options.insert("multiple", Variant::from(true));
        }
        PickMode::Folder => {
            options.insert("directory", Variant::from(true));
        }
        PickMode::OpenFile | PickMode::Save => {}
    }
    if !spec.filters.is_empty() {
        let filters: Vec<(String, Vec<(u32, String)>)> = spec.filters.iter().map(Filter::wire).collect();
        options.insert("current_filter", Variant::from(filters[0].clone()));
        options.insert("filters", Variant::from(filters));
    }
    if let Some(folder) = &spec.folder {
        options.insert("current_folder", Variant::from(byte_string(folder)));
    }
    if let Some(name) = &spec.name {
        options.insert("current_name", Variant::from(name.as_str()));
    }
    options
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Backend {
    Missing,
    Ski,
    Other,
}

impl Backend {
    pub fn from_names(portal_running: bool, activatable: &[String], owned: &[String]) -> Backend {
        let has = |name: &str| activatable.iter().chain(owned).any(|n| n == name);
        if !portal_running && !has(SERVICE) {
            return Backend::Missing;
        }
        if has(SKI_BACKEND) { Backend::Ski } else { Backend::Other }
    }

    pub fn describe(self) -> &'static str {
        match self {
            Backend::Missing => "no file chooser portal (xdg-desktop-portal is not available)",
            Backend::Ski => "file chooser portal available, Ski backend installed",
            Backend::Other => "file chooser portal available, Ski backend not installed",
        }
    }
}

pub async fn probe(bus: &BusKind) -> Backend {
    let Ok(connection) = bus.connect().await else {
        return Backend::Missing;
    };
    let Ok(proxy) = zbus::fdo::DBusProxy::new(&connection).await else {
        return Backend::Missing;
    };
    let portal_running = proxy.name_has_owner(SERVICE.try_into().unwrap()).await.unwrap_or(false);
    let activatable: Vec<String> =
        proxy.list_activatable_names().await.unwrap_or_default().iter().map(|n| n.to_string()).collect();
    let owned: Vec<String> = proxy.list_names().await.unwrap_or_default().iter().map(|n| n.to_string()).collect();
    Backend::from_names(portal_running, &activatable, &owned)
}

pub fn log_backend(bus: BusKind) {
    let spawned = std::thread::Builder::new().name("serp-portal-probe".to_owned()).spawn(move || {
        let backend = async_io::block_on(probe(&bus));
        match backend {
            Backend::Missing => tracing::warn!("{}", backend.describe()),
            _ => tracing::info!("{}", backend.describe()),
        }
    });
    if let Err(error) = spawned {
        tracing::warn!("portal probe could not start: {error}");
    }
}

fn unique_token() -> String {
    use std::sync::atomic::{AtomicU32, Ordering};
    static COUNTER: AtomicU32 = AtomicU32::new(0);
    format!("serp_{}_{}", std::process::id(), COUNTER.fetch_add(1, Ordering::Relaxed))
}

async fn request(bus: &BusKind, spec: &PickSpec, cancel: &async_channel::Receiver<()>) -> zbus::Result<Outcome> {
    let connection = bus.connect().await?;
    let unique = connection.unique_name().map(|name| name.to_string()).unwrap_or_default();
    let token = unique_token();
    let expected = request_path(&unique, &token);
    let request_proxy = zbus::Proxy::new(&connection, SERVICE, expected.as_str(), REQUEST).await?;
    let mut responses = request_proxy.receive_signal("Response").await?;
    let portal = zbus::Proxy::new(&connection, SERVICE, PATH, INTERFACE).await?;
    let method = if spec.mode == PickMode::Save { "SaveFile" } else { "OpenFile" };
    let options = options(spec, &token);
    let _handle: OwnedObjectPath = portal.call(method, &("", spec.title.as_str(), &options)).await?;
    enum Next {
        Response(Option<zbus::Message>),
        Cancel,
    }
    let next = async { Next::Response(responses.next().await) }.or(async {
        let _ = cancel.recv().await;
        Next::Cancel
    });
    match next.await {
        Next::Response(Some(message)) => {
            let (code, results): (u32, HashMap<String, OwnedVariant>) = message.body().deserialize()?;
            let uris = results
                .get("uris")
                .and_then(|value| <Vec<String>>::try_from(value.try_clone().ok()?).ok())
                .unwrap_or_default();
            Ok(outcome(code, &uris))
        }
        Next::Response(None) => Ok(Outcome::Cancelled),
        Next::Cancel => {
            let _: zbus::Result<()> = request_proxy.call("Close", &()).await;
            Ok(Outcome::Cancelled)
        }
    }
}

async fn run(bus: BusKind, spec: PickSpec, events: EventSink<Outcome>, commands: async_channel::Receiver<()>) {
    let result = match request(&bus, &spec, &commands).await {
        Ok(outcome) => outcome,
        Err(error) => {
            tracing::info!("file chooser portal unavailable: {error}");
            Outcome::Unavailable
        }
    };
    events.send(result);
}

pub struct Pick {
    _service: ServiceHandle<()>,
}

pub fn pick<D: 'static>(
    handle: &LoopHandle<'static, D>,
    bus: BusKind,
    spec: PickSpec,
    on_done: impl FnOnce(Outcome) + 'static,
) -> Result<Pick, IoError> {
    let mut on_done = Some(on_done);
    let service = spawn_worker(
        handle,
        "serp-file-chooser",
        move |outcome: Outcome| {
            if let Some(done) = on_done.take() {
                done(outcome);
            }
        },
        move |events, commands| run(bus, spec, events, commands),
    )?;
    Ok(Pick { _service: service })
}

#[cfg(test)]
mod tests {
    use std::sync::mpsc;
    use std::time::{Duration, Instant};

    use calloop::EventLoop;
    use zbus::ObjectPath;
    use zbus::interface;
    use zbus::message::Header;

    use super::*;
    use crate::platform::dbus::testbus::TestBus;

    struct Fake {
        code: u32,
        uris: Vec<String>,
        seen: mpsc::Sender<(String, Vec<String>)>,
        answer: bool,
    }

    #[interface(name = "org.freedesktop.portal.FileChooser")]
    impl Fake {
        async fn open_file(
            &self,
            #[zbus(header)] header: Header<'_>,
            #[zbus(connection)] connection: &zbus::Connection,
            parent: String,
            title: String,
            options: HashMap<String, OwnedVariant>,
        ) -> zbus::fdo::Result<OwnedObjectPath> {
            let _ = parent;
            let token: String = options
                .get("handle_token")
                .and_then(|v| String::try_from(v.try_clone().ok()?).ok())
                .unwrap_or_default();
            let mut keys: Vec<String> = options.keys().cloned().collect();
            keys.sort();
            let _ = self.seen.send((title, keys));
            let sender = header.sender().map(|s| s.to_string()).unwrap_or_default();
            let path = request_path(&sender, &token);
            if self.answer {
                let results = HashMap::from([(
                    "uris".to_owned(),
                    OwnedVariant::try_from(Variant::from(self.uris.clone())).unwrap(),
                )]);
                connection.emit_signal(None::<&str>, path.as_str(), REQUEST, "Response", &(self.code, results)).await?;
            }
            Ok(OwnedObjectPath::from(ObjectPath::try_from(path).unwrap()))
        }
    }

    fn serve(address: String, fake: Fake, ready: mpsc::Sender<()>) {
        std::thread::spawn(move || {
            async_io::block_on(async move {
                let _connection = zbus::connection::Builder::address(address.as_str())
                    .name(SERVICE)
                    .serve_at(PATH, fake)
                    .build()
                    .await
                    .unwrap();
                ready.send(()).unwrap();
                std::future::pending::<()>().await;
            });
        });
    }

    fn wait(event_loop: &mut EventLoop<'static, ()>, done: impl Fn() -> bool) -> bool {
        let deadline = Instant::now() + Duration::from_secs(10);
        while !done() && Instant::now() < deadline {
            event_loop.dispatch(Duration::from_millis(10), &mut ()).unwrap();
        }
        done()
    }

    #[test]
    fn request_paths_follow_the_portal_convention() {
        assert_eq!(request_path(":1.42", "tok"), "/org/freedesktop/portal/desktop/request/1_42/tok");
    }

    #[test]
    fn uris_decode_to_paths() {
        assert_eq!(uri_to_path("file:///home/a%20b/c.lrc"), Some(PathBuf::from("/home/a b/c.lrc")));
        assert_eq!(uri_to_path("file://host/x"), Some(PathBuf::from("/x")));
        assert_eq!(uri_to_path("https://x/y"), None);
        assert_eq!(uri_to_path("file://"), None);
        assert_eq!(percent_decode("%zz%41"), "%zzA");
        assert_eq!(percent_decode("%C3%A9"), "é");
    }

    #[test]
    fn backends_are_told_apart_by_bus_names() {
        let names = |list: &[&str]| list.iter().map(|n| (*n).to_owned()).collect::<Vec<_>>();
        assert_eq!(Backend::from_names(false, &[], &[]), Backend::Missing);
        assert_eq!(Backend::from_names(true, &[], &[]), Backend::Other);
        let ski = names(&["org.freedesktop.impl.portal.desktop.ski"]);
        assert_eq!(Backend::from_names(true, &ski, &[]), Backend::Ski);
        assert_eq!(Backend::from_names(false, &names(&[SERVICE]), &ski), Backend::Ski);
    }

    #[test]
    fn the_probe_sees_the_portal_on_the_bus() {
        let bus = TestBus::start();
        assert_eq!(async_io::block_on(probe(&bus.kind())), Backend::Missing);
        let (seen_tx, _seen_rx) = mpsc::channel();
        let (ready_tx, ready_rx) = mpsc::channel();
        serve(bus.address.clone(), Fake { code: 0, uris: Vec::new(), seen: seen_tx, answer: false }, ready_tx);
        ready_rx.recv_timeout(Duration::from_secs(10)).unwrap();
        assert_eq!(async_io::block_on(probe(&bus.kind())), Backend::Other);
    }

    #[test]
    fn paths_encode_to_uris_and_back() {
        let path = PathBuf::from("/a b/ü%.lrc");
        assert_eq!(path_to_uri(&path), "file:///a%20b/%C3%BC%25.lrc");
        assert_eq!(uri_to_path(&path_to_uri(&path)), Some(path));
    }

    #[test]
    fn response_codes_map_to_outcomes() {
        let uris = vec!["file:///a".to_owned(), "file:///b".to_owned()];
        assert_eq!(outcome(0, &uris), Outcome::Picked(vec![PathBuf::from("/a"), PathBuf::from("/b")]));
        assert_eq!(outcome(0, &[]), Outcome::Cancelled);
        assert_eq!(outcome(1, &uris), Outcome::Cancelled);
        assert_eq!(outcome(2, &uris), Outcome::Cancelled);
    }

    #[test]
    fn options_carry_mode_filters_and_folder() {
        let spec = PickSpec::new(PickMode::OpenFiles, "t")
            .filter(Filter::globs("Lyrics", &["*.lrc", "*.txt"]))
            .folder(PathBuf::from("/home/x"))
            .name("song.lrc");
        let options = options(&spec, "tok");
        let mut keys: Vec<_> = options.keys().copied().collect();
        keys.sort();
        assert_eq!(
            keys,
            ["current_filter", "current_folder", "current_name", "filters", "handle_token", "modal", "multiple"]
        );
        let folder = PickSpec::new(PickMode::Folder, "t");
        assert!(options_has(&folder, "directory"));
        assert!(!options_has(&PickSpec::new(PickMode::OpenFile, "t"), "directory"));
    }

    fn options_has(spec: &PickSpec, key: &str) -> bool {
        options(spec, "tok").contains_key(key)
    }

    #[test]
    fn the_portal_answer_arrives_as_paths() {
        let bus = TestBus::start();
        let (seen_tx, seen_rx) = mpsc::channel();
        let (ready_tx, ready_rx) = mpsc::channel();
        let fake = Fake { code: 0, uris: vec!["file:///music/a%20b.lrc".into()], seen: seen_tx, answer: true };
        serve(bus.address.clone(), fake, ready_tx);
        ready_rx.recv_timeout(Duration::from_secs(10)).unwrap();

        let mut event_loop: EventLoop<'static, ()> = EventLoop::try_new().unwrap();
        let result = std::rc::Rc::new(std::cell::RefCell::new(None));
        let sink = result.clone();
        let spec = PickSpec::new(PickMode::OpenFile, "Pick lyrics").filter(Filter::globs("Lyrics", &["*.lrc"]));
        let _pick =
            pick(&event_loop.handle(), bus.kind(), spec, move |outcome| *sink.borrow_mut() = Some(outcome)).unwrap();
        assert!(wait(&mut event_loop, || result.borrow().is_some()));
        assert_eq!(*result.borrow(), Some(Outcome::Picked(vec![PathBuf::from("/music/a b.lrc")])));
        let (title, keys) = seen_rx.recv_timeout(Duration::from_secs(5)).unwrap();
        assert_eq!(title, "Pick lyrics");
        assert!(keys.contains(&"filters".to_owned()) && keys.contains(&"handle_token".to_owned()));
    }

    #[test]
    fn a_cancelled_dialog_reports_cancelled() {
        let bus = TestBus::start();
        let (seen_tx, _seen_rx) = mpsc::channel();
        let (ready_tx, ready_rx) = mpsc::channel();
        serve(bus.address.clone(), Fake { code: 1, uris: Vec::new(), seen: seen_tx, answer: true }, ready_tx);
        ready_rx.recv_timeout(Duration::from_secs(10)).unwrap();

        let mut event_loop: EventLoop<'static, ()> = EventLoop::try_new().unwrap();
        let result = std::rc::Rc::new(std::cell::RefCell::new(None));
        let sink = result.clone();
        let spec = PickSpec::new(PickMode::OpenFile, "t");
        let _pick =
            pick(&event_loop.handle(), bus.kind(), spec, move |outcome| *sink.borrow_mut() = Some(outcome)).unwrap();
        assert!(wait(&mut event_loop, || result.borrow().is_some()));
        assert_eq!(*result.borrow(), Some(Outcome::Cancelled));
    }

    #[test]
    fn no_portal_reports_unavailable() {
        let bus = TestBus::start();
        let mut event_loop: EventLoop<'static, ()> = EventLoop::try_new().unwrap();
        let result = std::rc::Rc::new(std::cell::RefCell::new(None));
        let sink = result.clone();
        let spec = PickSpec::new(PickMode::OpenFile, "t");
        let _pick =
            pick(&event_loop.handle(), bus.kind(), spec, move |outcome| *sink.borrow_mut() = Some(outcome)).unwrap();
        assert!(wait(&mut event_loop, || result.borrow().is_some()));
        assert_eq!(*result.borrow(), Some(Outcome::Unavailable));
    }

    #[test]
    fn dropping_the_pick_closes_the_request() {
        let bus = TestBus::start();
        let (seen_tx, seen_rx) = mpsc::channel();
        let (ready_tx, ready_rx) = mpsc::channel();
        serve(bus.address.clone(), Fake { code: 0, uris: Vec::new(), seen: seen_tx, answer: false }, ready_tx);
        ready_rx.recv_timeout(Duration::from_secs(10)).unwrap();

        let mut event_loop: EventLoop<'static, ()> = EventLoop::try_new().unwrap();
        let called = std::rc::Rc::new(std::cell::Cell::new(false));
        let flag = called.clone();
        let spec = PickSpec::new(PickMode::OpenFile, "t");
        let handle = pick(&event_loop.handle(), bus.kind(), spec, move |_| flag.set(true)).unwrap();
        seen_rx.recv_timeout(Duration::from_secs(10)).unwrap();
        drop(handle);
        let end = Instant::now() + Duration::from_millis(300);
        while Instant::now() < end {
            event_loop.dispatch(Duration::from_millis(20), &mut ()).unwrap();
        }
        assert!(!called.get());
    }

    fn on_path(binary: &str) -> Option<std::path::PathBuf> {
        let path = std::env::var_os("PATH")?;
        std::env::split_paths(&path).map(|dir| dir.join(binary)).find(|candidate| candidate.is_file())
    }

    #[test]
    fn the_real_ski_chooser_opens_and_stays_alive_until_cancelled() {
        let Some(ski) = on_path("ski") else {
            eprintln!("skipped: ski is not on PATH");
            return;
        };
        if std::env::var_os("WAYLAND_DISPLAY").is_none() {
            eprintln!("skipped: no Wayland display");
            return;
        }
        let mut child = std::process::Command::new(ski)
            .args(["--chooser", "--mode", "open"])
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null())
            .spawn()
            .expect("ski starts");
        std::thread::sleep(Duration::from_millis(1500));
        let running = child.try_wait().expect("status").is_none();
        let _ = child.kill();
        let status = child.wait().expect("reaped");
        assert!(running, "ski chooser exited on its own with {status:?}");
    }
}
