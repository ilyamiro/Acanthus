mod hyprland;
mod lines;
mod mango;
mod niri;
mod sway;
mod types;

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::Duration;

use acanthus_core::{Signal, batch};
use acanthus_io::IoError;
use calloop::LoopHandle;
use serde_json::Value;

use crate::platform::bridge::{EventSink, StopFlag, ThreadHandle, spawn_thread};
use hyprland::HyprlandPaths;
use lines::LineReader;
use mango::MangoModel;
use niri::NiriModel;
use sway::SwayConnection;
pub use types::{CompositorKind, Snapshot, Window, Workspace};

const POLL: Duration = Duration::from_millis(40);
const RECONNECT: Duration = Duration::from_secs(1);

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct FocusInfo {
    pub app_class: String,
    pub title: String,
}

impl FocusInfo {
    pub fn display_text(&self) -> &str {
        if self.title.is_empty() { &self.app_class } else { &self.title }
    }

    pub fn is_focused(&self) -> bool {
        !self.display_text().is_empty()
    }
}

enum Message {
    Snapshot(Snapshot),
    MangoTags(Value),
    MangoClients(Value),
    Lost,
}

#[derive(Debug, Clone)]
enum Endpoint {
    Hyprland(HyprlandPaths),
    Niri(PathBuf),
    Sway(PathBuf),
    Mango,
    None,
}

pub struct Compositor {
    pub kind: CompositorKind,
    pub snapshot: Signal<Snapshot>,
    pub focus: Signal<FocusInfo>,
    pub connected: Signal<bool>,
    endpoint: Endpoint,
    _thread: Option<ThreadHandle>,
}

fn apply(snapshot: &Signal<Snapshot>, focus: &Signal<FocusInfo>, connected: &Signal<bool>, next: Snapshot) {
    let info = next
        .focused_window()
        .map(|w| FocusInfo { app_class: w.app_id.clone(), title: w.title.clone() })
        .unwrap_or_default();
    batch(|| {
        connected.set(true);
        focus.set(info);
        snapshot.set(next);
    });
}

fn endpoint_for(kind: CompositorKind, env: &impl Fn(&str) -> Option<String>) -> Endpoint {
    match kind {
        CompositorKind::Hyprland => HyprlandPaths::from_env(env).map_or(Endpoint::None, Endpoint::Hyprland),
        CompositorKind::Niri => {
            env("NIRI_SOCKET").filter(|s| !s.is_empty()).map_or(Endpoint::None, |s| Endpoint::Niri(s.into()))
        }
        CompositorKind::Sway => {
            env("SWAYSOCK").filter(|s| !s.is_empty()).map_or(Endpoint::None, |s| Endpoint::Sway(s.into()))
        }
        CompositorKind::Mango => Endpoint::Mango,
        CompositorKind::Unknown => Endpoint::None,
    }
}

impl Compositor {
    pub fn start<D: 'static>(
        handle: &LoopHandle<'static, D>,
        env: impl Fn(&str) -> Option<String>,
    ) -> Result<Compositor, IoError> {
        let kind = CompositorKind::detect(&env);
        let endpoint = endpoint_for(kind, &env);
        let snapshot = Signal::new(Snapshot::default());
        let focus = Signal::new(FocusInfo::default());
        let connected = Signal::new(false);

        let thread = match endpoint.clone() {
            Endpoint::None => None,
            endpoint => {
                let (s, f, c) = (snapshot.clone(), focus.clone(), connected.clone());
                let mut mango = MangoModel::default();
                Some(spawn_thread(
                    handle,
                    "serp-compositor",
                    move |message: Message| match message {
                        Message::Snapshot(next) => apply(&s, &f, &c, next),
                        Message::MangoTags(tags) => {
                            mango.set_tags(tags);
                            apply(&s, &f, &c, mango.snapshot());
                        }
                        Message::MangoClients(clients) => {
                            mango.set_clients(clients);
                            apply(&s, &f, &c, mango.snapshot());
                        }
                        Message::Lost => c.set(false),
                    },
                    move |events, stop| match endpoint {
                        Endpoint::Hyprland(paths) => run_hyprland(&paths, &events, &stop),
                        Endpoint::Niri(socket) => run_niri(&socket, &events, &stop),
                        Endpoint::Sway(socket) => run_sway(&socket, &events, &stop),
                        Endpoint::Mango => run_mango(&events, &stop),
                        Endpoint::None => {}
                    },
                )?)
            }
        };
        Ok(Compositor { kind, snapshot, focus, connected, endpoint, _thread: thread })
    }

    pub fn focus_workspace(&self, index: u32) {
        let endpoint = self.endpoint.clone();
        std::thread::spawn(move || {
            let result = match &endpoint {
                Endpoint::Hyprland(p) => hyprland::focus_workspace(&p.requests, i64::from(index) + 1).map(|_| ()),
                Endpoint::Niri(socket) => niri_request(socket, &niri::focus_workspace_request(index + 1)),
                Endpoint::Sway(socket) => sway_command(socket, &format!("workspace number {}", index + 1)),
                Endpoint::Mango => run_detached(mango::view_command(index)),
                Endpoint::None => Ok(()),
            };
            log_failure("focus workspace", result);
        });
    }

    pub fn focus_window(&self, window: &Window) {
        let (endpoint, id) = (self.endpoint.clone(), window.native_id.clone());
        std::thread::spawn(move || {
            let result = match &endpoint {
                Endpoint::Hyprland(p) => hyprland::focus_window(&p.requests, &id).map(|_| ()),
                Endpoint::Niri(socket) => match id.parse::<i64>() {
                    Ok(n) => niri_request(socket, &niri::focus_window_request(n)),
                    Err(_) => Ok(()),
                },
                Endpoint::Sway(socket) => sway_command(socket, &format!("[con_id={id}] focus")),
                Endpoint::Mango => run_detached(mango::focus_command(&id)),
                Endpoint::None => Ok(()),
            };
            log_failure("focus window", result);
        });
    }

    pub fn close_window(&self, window: &Window) {
        let (endpoint, id) = (self.endpoint.clone(), window.native_id.clone());
        std::thread::spawn(move || {
            let result = match &endpoint {
                Endpoint::Hyprland(p) => hyprland::close_window(&p.requests, &id).map(|_| ()),
                Endpoint::Niri(socket) => match id.parse::<i64>() {
                    Ok(n) => niri_request(socket, &niri::close_window_request(n)),
                    Err(_) => Ok(()),
                },
                Endpoint::Sway(socket) => sway_command(socket, &format!("[con_id={id}] kill")),
                Endpoint::Mango => run_detached(mango::close_command(&id)),
                Endpoint::None => Ok(()),
            };
            log_failure("close window", result);
        });
    }

    pub fn switch_keyboard_layout(&self) {
        let endpoint = self.endpoint.clone();
        std::thread::spawn(move || {
            let result = match &endpoint {
                Endpoint::Hyprland(p) => hyprland::switch_keyboard_layout(&p.requests).map(|_| ()),
                Endpoint::Niri(socket) => {
                    niri_request(socket, &serde_json::json!({"Action": {"SwitchLayout": {"layout": "Next"}}}))
                }
                Endpoint::Sway(socket) => sway_command(socket, "input type:keyboard xkb_switch_layout next"),
                Endpoint::Mango | Endpoint::None => Ok(()),
            };
            log_failure("switch keyboard layout", result);
        });
    }
}

fn log_failure(action: &str, result: std::io::Result<()>) {
    if let Err(e) = result {
        tracing::warn!("compositor action '{action}' failed: {e}");
    }
}

fn run_detached(argv: Vec<String>) -> std::io::Result<()> {
    acanthus_io::exec_detached(argv).map_err(|e| std::io::Error::other(e.to_string()))
}

fn sway_command(socket: &std::path::Path, command: &str) -> std::io::Result<()> {
    SwayConnection::connect(socket, Duration::from_millis(200))?.run(command).map(|_| ())
}

fn niri_request(socket: &std::path::Path, request: &Value) -> std::io::Result<()> {
    let mut stream = UnixStream::connect(socket)?;
    stream.set_read_timeout(Some(Duration::from_secs(2)))?;
    stream.write_all(format!("{request}\n").as_bytes())?;
    let mut reply = String::new();
    BufReader::new(stream).read_line(&mut reply)?;
    if reply.contains("\"Err\"") { Err(std::io::Error::other(reply.trim().to_owned())) } else { Ok(()) }
}

fn run_hyprland(paths: &HyprlandPaths, events: &EventSink<Message>, stop: &StopFlag) {
    while !stop.is_set() {
        match hyprland::fetch(&paths.requests) {
            Ok(snapshot) => {
                events.send(Message::Snapshot(snapshot));
            }
            Err(e) => {
                tracing::debug!("hyprland not reachable: {e}");
                events.send(Message::Lost);
                if stop.sleep(RECONNECT) {
                    return;
                }
                continue;
            }
        }
        if let Ok(mut reader) = hyprland::connect_events(&paths.events, POLL) {
            let mut dirty = false;
            loop {
                if stop.is_set() {
                    return;
                }
                match reader.next_line() {
                    Ok(Some(_)) => dirty = true,
                    Ok(None) if dirty => {
                        dirty = false;
                        match hyprland::fetch(&paths.requests) {
                            Ok(snapshot) => {
                                events.send(Message::Snapshot(snapshot));
                            }
                            Err(_) => break,
                        }
                    }
                    Ok(None) => {}
                    Err(_) => break,
                }
            }
        }
        events.send(Message::Lost);
        if stop.sleep(RECONNECT) {
            return;
        }
    }
}

fn run_niri(socket: &std::path::Path, events: &EventSink<Message>, stop: &StopFlag) {
    while !stop.is_set() {
        let session = (|| -> std::io::Result<LineReader> {
            let mut stream = UnixStream::connect(socket)?;
            stream.write_all(b"\"EventStream\"\n")?;
            LineReader::new(stream, POLL)
        })();
        if let Ok(mut reader) = session {
            let mut model = NiriModel::default();
            let mut dirty = false;
            loop {
                if stop.is_set() {
                    return;
                }
                match reader.next_line() {
                    Ok(Some(line)) => {
                        if let Ok(event) = serde_json::from_str::<Value>(&line) {
                            dirty |= model.apply(&event);
                        }
                    }
                    Ok(None) if dirty => {
                        dirty = false;
                        events.send(Message::Snapshot(model.snapshot()));
                    }
                    Ok(None) => {}
                    Err(_) => break,
                }
            }
        }
        events.send(Message::Lost);
        if stop.sleep(RECONNECT) {
            return;
        }
    }
}

fn run_sway(socket: &std::path::Path, events: &EventSink<Message>, stop: &StopFlag) {
    while !stop.is_set() {
        let session = (|| -> std::io::Result<(SwayConnection, SwayConnection)> {
            let mut commands = SwayConnection::connect(socket, Duration::from_millis(500))?;
            let mut subscription = SwayConnection::connect(socket, POLL)?;
            subscription.subscribe(&["window", "workspace", "input"])?;
            events.send(Message::Snapshot(commands.fetch()?));
            Ok((commands, subscription))
        })();
        if let Ok((mut commands, mut subscription)) = session {
            let mut dirty = false;
            loop {
                if stop.is_set() {
                    return;
                }
                match subscription.next_event() {
                    Ok(Some(_)) => dirty = true,
                    Ok(None) if dirty => {
                        dirty = false;
                        match commands.fetch() {
                            Ok(snapshot) => {
                                events.send(Message::Snapshot(snapshot));
                            }
                            Err(_) => break,
                        }
                    }
                    Ok(None) => {}
                    Err(_) => break,
                }
            }
        }
        events.send(Message::Lost);
        if stop.sleep(RECONNECT) {
            return;
        }
    }
}

fn run_mango(events: &EventSink<Message>, stop: &StopFlag) {
    let spawn_stream = |topic: &'static str, wrap: fn(Value) -> Message| {
        let events = events.clone();
        let stop = stop.clone();
        std::thread::spawn(move || {
            while !stop.is_set() {
                let child = Command::new("mmsg")
                    .args(["watch", topic])
                    .stdin(Stdio::null())
                    .stdout(Stdio::piped())
                    .stderr(Stdio::null())
                    .spawn();
                if let Ok(mut child) = child {
                    let stdout = child.stdout.take();
                    let killer_stop = stop.clone();
                    let pid = child.id();
                    let guard = std::thread::spawn(move || {
                        while !killer_stop.is_set() {
                            std::thread::sleep(Duration::from_millis(100));
                        }
                        if let Some(pid) = rustix::process::Pid::from_raw(pid as i32) {
                            let _ = rustix::process::kill_process(pid, rustix::process::Signal::TERM);
                        }
                    });
                    if let Some(stdout) = stdout {
                        for line in BufReader::new(stdout).lines().map_while(Result::ok) {
                            if let Ok(value) = serde_json::from_str::<Value>(line.trim()) {
                                events.send(wrap(value));
                            }
                        }
                    }
                    let _ = child.wait();
                    drop(guard);
                }
                if stop.sleep(RECONNECT) {
                    return;
                }
            }
        })
    };
    let tags = spawn_stream("all-tags", Message::MangoTags);
    let clients = spawn_stream("all-clients", Message::MangoClients);
    while !stop.sleep(Duration::from_millis(200)) {}
    let _ = tags.join();
    let _ = clients.join();
}

#[cfg(test)]
mod tests {
    use std::io::Write;
    use std::os::unix::net::UnixListener;
    use std::sync::{Arc, Mutex};
    use std::time::Instant;

    use calloop::EventLoop;
    use serde_json::json;

    use super::*;

    fn pump_until(event_loop: &mut EventLoop<'static, ()>, done: impl Fn() -> bool) -> bool {
        let deadline = Instant::now() + Duration::from_secs(10);
        while !done() && Instant::now() < deadline {
            event_loop.dispatch(Duration::from_millis(10), &mut ()).unwrap();
        }
        done()
    }

    fn env_map(pairs: Vec<(String, String)>) -> impl Fn(&str) -> Option<String> {
        move |k| pairs.iter().find(|(key, _)| key == k).map(|(_, v)| v.clone())
    }

    #[test]
    fn hyprland_snapshot_follows_events_and_actions_reach_the_socket() {
        let fake = hyprland::tests::FakeHyprland::start();
        let env = env_map(vec![
            ("HYPRLAND_INSTANCE_SIGNATURE".into(), fake.signature.clone()),
            ("XDG_RUNTIME_DIR".into(), fake.dir.path().to_string_lossy().into_owned()),
        ]);
        let mut event_loop: EventLoop<'static, ()> = EventLoop::try_new().unwrap();
        let compositor = Compositor::start(&event_loop.handle(), env).unwrap();
        assert_eq!(compositor.kind, CompositorKind::Hyprland);

        let snapshot = compositor.snapshot.clone();
        assert!(pump_until(&mut event_loop, || !snapshot.get_untracked().windows.is_empty()));
        assert!(compositor.connected.get_untracked());
        assert_eq!(compositor.focus.get_untracked().display_text(), "term");
        assert_eq!(compositor.snapshot.get_untracked().active_index(), 2);

        *fake.active.lock().unwrap() = json!({"address": "0x64e79f243580"});
        fake.push_event("activewindowv2>>64e79f243580");
        let focus = compositor.focus.clone();
        assert!(pump_until(&mut event_loop, || focus.get_untracked().app_class == "firefox"));

        compositor.focus_workspace(1);
        let log = fake.log.clone();
        let deadline = Instant::now() + Duration::from_secs(5);
        while !log.lock().unwrap().iter().any(|l| l.contains("workspace = 2")) && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(fake.log.lock().unwrap().iter().any(|l| l == "dispatch hl.dsp.focus({ workspace = 2 })"));
    }

    #[test]
    fn niri_event_stream_builds_the_snapshot() {
        let dir = tempfile::tempdir().unwrap();
        let socket = dir.path().join("niri.sock");
        let listener = UnixListener::bind(&socket).unwrap();
        let received = Arc::new(Mutex::new(Vec::<String>::new()));
        let log = received.clone();
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { break };
                let mut line = String::new();
                let mut reader = BufReader::new(stream.try_clone().unwrap());
                let _ = reader.read_line(&mut line);
                log.lock().unwrap().push(line.trim().to_owned());
                if line.contains("EventStream") {
                    let events = [
                        json!({"Ok": "Handled"}),
                        json!({"WorkspacesChanged": {"workspaces": [{"id": 1, "idx": 1, "output": "eDP-1", "is_active": true, "is_focused": true, "active_window_id": 5}]}}),
                        json!({"WindowsChanged": {"windows": [{"id": 5, "title": "term", "app_id": "foot", "workspace_id": 1, "is_focused": true}]}}),
                        json!({"KeyboardLayoutsChanged": {"keyboard_layouts": {"names": ["English (US)"], "current_idx": 0}}}),
                    ];
                    for event in events {
                        let _ = stream.write_all(format!("{event}\n").as_bytes());
                    }
                    std::thread::sleep(Duration::from_secs(3));
                } else {
                    let _ = stream.write_all(b"{\"Ok\":\"Handled\"}\n");
                }
            }
        });

        let env = env_map(vec![("NIRI_SOCKET".into(), socket.to_string_lossy().into_owned())]);
        let mut event_loop: EventLoop<'static, ()> = EventLoop::try_new().unwrap();
        let compositor = Compositor::start(&event_loop.handle(), env).unwrap();
        assert_eq!(compositor.kind, CompositorKind::Niri);
        let focus = compositor.focus.clone();
        assert!(pump_until(&mut event_loop, || focus.get_untracked().app_class == "foot"));
        let snapshot = compositor.snapshot.get_untracked();
        assert_eq!(snapshot.workspaces.len(), 1);
        assert!(snapshot.workspaces[0].occupied);
        assert_eq!(snapshot.short_keyboard_layout(), "EN");

        compositor.focus_workspace(2);
        let deadline = Instant::now() + Duration::from_secs(5);
        while !received.lock().unwrap().iter().any(|l| l.contains("FocusWorkspace")) && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(10));
        }
        let sent = received.lock().unwrap().iter().find(|l| l.contains("FocusWorkspace")).cloned().unwrap();
        assert_eq!(serde_json::from_str::<Value>(&sent).unwrap(), niri::focus_workspace_request(3));
    }

    #[test]
    fn sway_backend_fetches_and_refreshes_on_events() {
        let dir = tempfile::tempdir().unwrap();
        let socket = dir.path().join("sway.sock");
        sway::tests::serve(socket.clone());
        let env = env_map(vec![("SWAYSOCK".into(), socket.to_string_lossy().into_owned())]);
        let mut event_loop: EventLoop<'static, ()> = EventLoop::try_new().unwrap();
        let compositor = Compositor::start(&event_loop.handle(), env).unwrap();
        assert_eq!(compositor.kind, CompositorKind::Sway);
        let snapshot = compositor.snapshot.clone();
        assert!(pump_until(&mut event_loop, || snapshot.get_untracked().windows.len() == 4));
        assert_eq!(compositor.focus.get_untracked().display_text(), "shell");
    }

    #[test]
    fn unknown_compositor_stays_disconnected() {
        let event_loop: EventLoop<'static, ()> = EventLoop::try_new().unwrap();
        let compositor = Compositor::start(&event_loop.handle(), |_| None).unwrap();
        assert_eq!(compositor.kind, CompositorKind::Unknown);
        assert!(!compositor.connected.get_untracked());
        compositor.focus_workspace(0);
    }

    #[test]
    fn focus_info_prefers_the_title() {
        let info = FocusInfo { app_class: "kitty".into(), title: String::new() };
        assert_eq!(info.display_text(), "kitty");
        let info = FocusInfo { app_class: "kitty".into(), title: "vim".into() };
        assert_eq!(info.display_text(), "vim");
        assert!(!FocusInfo::default().is_focused());
    }
}
