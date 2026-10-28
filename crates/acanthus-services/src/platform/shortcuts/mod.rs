mod chord;
mod portal;

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use acanthus_core::Signal;
use acanthus_io::IoError;
use calloop::LoopHandle;

pub use chord::{Chord, ChordError, Modifiers};

use crate::platform::bridge::{ServiceHandle, ThreadHandle, spawn_supervised, spawn_thread};
use crate::platform::compositor::{CompositorKind, hyprland_command, hyprland_socket, sway_run, sway_socket};
use crate::platform::dbus::BusKind;

/// One global shortcut: a stable id, a description for the desktop's settings dialog and the chord it asks for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Shortcut {
    /// Stable identifier; reported back on activation and substituted for `{id}` in the [`Launcher`].
    pub id: String,
    /// What the shortcut does, in the user's language.
    pub description: String,
    /// The requested key combination.
    pub chord: Chord,
}

impl Shortcut {
    /// A shortcut with the chord parsed from `chord`.
    pub fn new(id: &str, description: &str, chord: &str) -> Result<Shortcut, ChordError> {
        Ok(Shortcut { id: id.to_owned(), description: description.to_owned(), chord: Chord::parse(chord)? })
    }
}

/// The command a compositor binding runs to deliver a shortcut to the application, usually its own command-line client calling an IPC action. `{id}` in any argument becomes the shortcut id.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Launcher {
    argv: Vec<String>,
}

impl Launcher {
    /// A launcher from its argument vector.
    pub fn new<S: Into<String>>(argv: impl IntoIterator<Item = S>) -> Launcher {
        Launcher { argv: argv.into_iter().map(Into::into).collect() }
    }

    /// The argument vector for the shortcut `id`.
    pub fn argv(&self, id: &str) -> Vec<String> {
        self.argv.iter().map(|arg| arg.replace("{id}", id)).collect()
    }

    /// The command line for `id`, quoted for a POSIX shell.
    pub fn shell(&self, id: &str) -> String {
        self.argv(id).iter().map(|arg| quote(arg)).collect::<Vec<_>>().join(" ")
    }
}

fn quote(arg: &str) -> String {
    let plain = !arg.is_empty() && arg.chars().all(|c| c.is_ascii_alphanumeric() || "-_./:=@%+".contains(c));
    if plain { arg.to_owned() } else { format!("'{}'", arg.replace('\'', "'\\''")) }
}

/// How shortcuts reach the application.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Strategy {
    /// Bindings added to a running Hyprland over its request socket; each runs the [`Launcher`].
    Hyprland(PathBuf),
    /// Bindings added to a running Sway over its IPC socket; each runs the [`Launcher`].
    Sway(PathBuf),
    /// The XDG GlobalShortcuts portal; activations arrive as signals. Falls back to a configuration snippet when the portal lacks the interface.
    Portal,
}

impl Strategy {
    /// Hyprland or Sway when their sockets are in `env`, the portal otherwise or when `prefer_portal` is set.
    pub fn choose(env: impl Fn(&str) -> Option<String>, prefer_portal: bool) -> Strategy {
        if prefer_portal {
            return Strategy::Portal;
        }
        if let Some(socket) = hyprland_socket(&env) {
            return Strategy::Hyprland(socket);
        }
        if let Some(socket) = sway_socket(&env) {
            return Strategy::Sway(socket);
        }
        Strategy::Portal
    }
}

/// How the shortcuts ended up registered.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Method {
    /// Bound in Hyprland.
    Hyprland,
    /// Bound in Sway.
    Sway,
    /// Bound through the portal.
    Portal,
}

/// Where registration stands.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Registration {
    /// Registration is in progress.
    Pending,
    /// Every shortcut is bound.
    Bound(Method),
    /// The compositor offers no runtime registration; the user adds [`ShortcutsState::snippet`] to its configuration.
    Manual,
    /// Registration failed with this message.
    Failed(String),
}

/// The configuration lines that bind `shortcuts` to `launcher` in the compositor `kind`, for compositors without runtime registration and as a fallback the user can copy.
pub fn snippet(kind: CompositorKind, shortcuts: &[Shortcut], launcher: &Launcher) -> String {
    let lines: Vec<String> = shortcuts
        .iter()
        .map(|s| {
            let command = launcher.shell(&s.id);
            match kind {
                CompositorKind::Hyprland => format!("bind = {}, exec, {command}", s.chord.hyprland()),
                CompositorKind::Sway => format!("bindsym {} exec {command}", s.chord.sway()),
                CompositorKind::Niri => {
                    let argv: Vec<String> = launcher.argv(&s.id).iter().map(|a| format!("{a:?}")).collect();
                    format!("    {} {{ spawn {}; }}", s.chord.niri(), argv.join(" "))
                }
                CompositorKind::Mango => format!("bind={},spawn,{command}", s.chord.mango()),
                CompositorKind::Unknown => format!("{}: {command}", s.chord),
            }
        })
        .collect();
    match kind {
        CompositorKind::Niri => format!("binds {{\n{}\n}}", lines.join("\n")),
        _ => lines.join("\n"),
    }
}

enum Event {
    Done(Result<Method, String>),
    Portal(portal::Outcome),
    Activated(String),
}

/// Observable state of the shortcut registration.
#[derive(Clone)]
pub struct ShortcutsState {
    /// Where registration stands.
    pub registration: Signal<Registration>,
    /// The configuration snippet for the detected compositor; filled when registration is [`Registration::Manual`] or failed.
    pub snippet: Signal<String>,
}

/// What to register and how to detect the environment.
pub struct ShortcutsConfig {
    /// The shortcuts to register.
    pub shortcuts: Vec<Shortcut>,
    /// The command compositor bindings run.
    pub launcher: Launcher,
    /// The detected compositor, used for the snippet.
    pub kind: CompositorKind,
    /// How shortcuts reach the application.
    pub strategy: Strategy,
    /// The session bus, for the portal.
    pub bus: BusKind,
}

#[derive(Default)]
enum Binding {
    Hyprland(PathBuf, Vec<String>),
    Sway(PathBuf, Vec<String>),
    #[default]
    None,
}

impl Binding {
    fn release(&self) {
        match self {
            Binding::Hyprland(socket, lines) => {
                for line in lines {
                    let _ = hyprland_command(socket, &format!("keyword unbind {line}"));
                }
            }
            Binding::Sway(socket, lines) => {
                for line in lines {
                    let _ = sway_run(socket, &format!("unbindsym {line}"));
                }
            }
            Binding::None => {}
        }
    }
}

fn bind_all(strategy: &Strategy, shortcuts: &[Shortcut], launcher: &Launcher) -> Result<(Method, Binding), String> {
    match strategy {
        Strategy::Hyprland(socket) => {
            let mut done = Vec::new();
            for s in shortcuts {
                let keys = s.chord.hyprland();
                let line = format!("keyword bind {keys}, exec, {}", launcher.shell(&s.id));
                match hyprland_command(socket, &line) {
                    Ok(true) => done.push(keys),
                    Ok(false) => {
                        Binding::Hyprland(socket.clone(), done).release();
                        return Err(format!("Hyprland rejected the binding for `{}`", s.id));
                    }
                    Err(e) => {
                        Binding::Hyprland(socket.clone(), done).release();
                        return Err(e.to_string());
                    }
                }
            }
            Ok((Method::Hyprland, Binding::Hyprland(socket.clone(), done)))
        }
        Strategy::Sway(socket) => {
            let mut done = Vec::new();
            for s in shortcuts {
                let keys = s.chord.sway();
                let line = format!("bindsym {keys} exec {}", launcher.shell(&s.id));
                match sway_run(socket, &line) {
                    Ok(true) => done.push(keys),
                    Ok(false) => {
                        Binding::Sway(socket.clone(), done).release();
                        return Err(format!("Sway rejected the binding for `{}`", s.id));
                    }
                    Err(e) => {
                        Binding::Sway(socket.clone(), done).release();
                        return Err(e.to_string());
                    }
                }
            }
            Ok((Method::Sway, Binding::Sway(socket.clone(), done)))
        }
        Strategy::Portal => Err("portal".to_owned()),
    }
}

/// Global shortcuts registered with the compositor or the portal. Bindings added to Hyprland or Sway are removed when this is dropped; the portal session ends with the connection.
pub struct Shortcuts {
    /// Its observable state.
    pub state: ShortcutsState,
    binding: Arc<Mutex<Binding>>,
    _thread: Option<ThreadHandle>,
    _portal: Option<ServiceHandle<()>>,
}

impl Drop for Shortcuts {
    fn drop(&mut self) {
        let binding = std::mem::replace(&mut *self.binding.lock().unwrap_or_else(|e| e.into_inner()), Binding::None);
        binding.release();
    }
}

impl Shortcuts {
    /// Registers `config.shortcuts`; `on_activate` receives the id of each shortcut the portal reports (compositor bindings run the launcher instead).
    pub fn start<D: 'static>(
        handle: &LoopHandle<'static, D>,
        config: ShortcutsConfig,
        mut on_activate: impl FnMut(&str) + 'static,
    ) -> Result<Shortcuts, IoError> {
        let state =
            ShortcutsState { registration: Signal::new(Registration::Pending), snippet: Signal::new(String::new()) };
        let ShortcutsConfig { shortcuts, launcher, kind, strategy, bus } = config;
        let target = state.clone();
        let (for_snippet, launch) = (shortcuts.clone(), launcher.clone());
        let publish_snippet = move |target: &ShortcutsState| {
            target.snippet.set(snippet(kind, &for_snippet, &launch));
        };
        let on_event = move |event: Event| match event {
            Event::Done(Ok(method)) => target.registration.set(Registration::Bound(method)),
            Event::Done(Err(message)) => {
                publish_snippet(&target);
                target.registration.set(Registration::Failed(message));
            }
            Event::Portal(portal::Outcome::Bound) => target.registration.set(Registration::Bound(Method::Portal)),
            Event::Portal(portal::Outcome::Unavailable) => {
                publish_snippet(&target);
                target.registration.set(Registration::Manual);
            }
            Event::Activated(id) => on_activate(&id),
        };
        match strategy {
            Strategy::Portal => {
                let portal = spawn_supervised(
                    handle,
                    "acanthus-shortcuts",
                    on_event,
                    move |events, _commands: async_channel::Receiver<()>| {
                        portal::run(bus.clone(), shortcuts.clone(), events)
                    },
                )?;
                Ok(Shortcuts { state, binding: Arc::default(), _thread: None, _portal: Some(portal) })
            }
            strategy => {
                let slot = Arc::new(Mutex::new(Binding::None));
                let slot_worker = slot.clone();
                let thread =
                    spawn_thread(handle, "acanthus-shortcuts", on_event, move |events, _stop| {
                        match bind_all(&strategy, &shortcuts, &launcher) {
                            Ok((method, binding)) => {
                                *slot_worker.lock().unwrap_or_else(|e| e.into_inner()) = binding;
                                events.send(Event::Done(Ok(method)));
                            }
                            Err(message) => {
                                events.send(Event::Done(Err(message)));
                            }
                        }
                    })?;
                Ok(Shortcuts { state, binding: slot, _thread: Some(thread), _portal: None })
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::sync::mpsc;
    use std::time::{Duration, Instant};

    use calloop::EventLoop;
    use zbus::interface;
    use zbus::object_server::SignalEmitter;
    use zbus::{OwnedObjectPath, OwnedVariant, Variant};

    use super::*;
    use crate::platform::compositor::{FakeHyprland, serve_fake_sway};
    use crate::platform::dbus::testbus::TestBus;

    fn sample() -> Vec<Shortcut> {
        vec![
            Shortcut::new("toggle-launcher", "Toggle the launcher", "Super+Space").unwrap(),
            Shortcut::new("lock", "Lock the screen", "Super+Shift+L").unwrap(),
        ]
    }

    fn launcher() -> Launcher {
        Launcher::new(["app", "action", "{id}"])
    }

    fn pump_until(event_loop: &mut EventLoop<'static, ()>, done: impl Fn() -> bool) -> bool {
        let deadline = Instant::now() + Duration::from_secs(10);
        while !done() && Instant::now() < deadline {
            event_loop.dispatch(Duration::from_millis(10), &mut ()).unwrap();
        }
        done()
    }

    #[test]
    fn launcher_substitutes_the_id_and_quotes_for_the_shell() {
        assert_eq!(launcher().argv("lock"), ["app", "action", "lock"]);
        assert_eq!(launcher().shell("lock"), "app action lock");
        assert_eq!(Launcher::new(["my app", "it's {id}", ""]).shell("x"), "'my app' 'it'\\''s x' ''");
    }

    #[test]
    fn strategy_prefers_compositor_sockets_then_the_portal() {
        let fake = FakeHyprland::start();
        let runtime = fake.dir.path().to_string_lossy().into_owned();
        let signature = fake.signature.clone();
        let hypr = move |key: &str| match key {
            "HYPRLAND_INSTANCE_SIGNATURE" => Some(signature.clone()),
            "XDG_RUNTIME_DIR" => Some(runtime.clone()),
            _ => None,
        };
        assert_eq!(Strategy::choose(&hypr, false), Strategy::Hyprland(fake.requests.clone()));
        assert_eq!(Strategy::choose(&hypr, true), Strategy::Portal);
        let sway = |key: &str| (key == "SWAYSOCK").then(|| "/run/sway.sock".to_owned());
        assert_eq!(Strategy::choose(sway, false), Strategy::Sway("/run/sway.sock".into()));
        assert_eq!(Strategy::choose(|_| None, false), Strategy::Portal);
    }

    #[test]
    fn snippets_follow_each_compositor_syntax() {
        let shortcuts = sample();
        let hypr = snippet(CompositorKind::Hyprland, &shortcuts, &launcher());
        assert_eq!(hypr.lines().next(), Some("bind = SUPER, Space, exec, app action toggle-launcher"));
        assert!(hypr.contains("bind = SUPER SHIFT, L, exec, app action lock"));
        let sway = snippet(CompositorKind::Sway, &shortcuts, &launcher());
        assert_eq!(sway.lines().next(), Some("bindsym Mod4+Space exec app action toggle-launcher"));
        let niri = snippet(CompositorKind::Niri, &shortcuts, &launcher());
        assert!(niri.starts_with("binds {\n"));
        assert!(niri.contains("Mod+Shift+L { spawn \"app\" \"action\" \"lock\"; }"));
        let mango = snippet(CompositorKind::Mango, &shortcuts, &launcher());
        assert!(mango.contains("bind=SUPER+SHIFT,l,spawn,app action lock"));
        let other = snippet(CompositorKind::Unknown, &shortcuts, &launcher());
        assert!(other.contains("Super+Shift+L: app action lock"));
    }

    #[test]
    fn hyprland_bindings_are_added_and_removed_on_drop() {
        let fake = FakeHyprland::start();
        let mut event_loop: EventLoop<'static, ()> = EventLoop::try_new().unwrap();
        let config = ShortcutsConfig {
            shortcuts: sample(),
            launcher: launcher(),
            kind: CompositorKind::Hyprland,
            strategy: Strategy::Hyprland(fake.requests.clone()),
            bus: BusKind::Address(String::new()),
        };
        let shortcuts = Shortcuts::start(&event_loop.handle(), config, |_| {}).unwrap();
        let state = shortcuts.state.clone();
        assert!(pump_until(&mut event_loop, || state.registration.get_untracked() != Registration::Pending));
        assert_eq!(state.registration.get_untracked(), Registration::Bound(Method::Hyprland));
        {
            let log = fake.log.lock().unwrap();
            assert!(log.contains(&"keyword bind SUPER, Space, exec, app action toggle-launcher".to_owned()));
            assert!(log.contains(&"keyword bind SUPER SHIFT, L, exec, app action lock".to_owned()));
        }
        drop(shortcuts);
        let log = fake.log.lock().unwrap();
        assert!(log.contains(&"keyword unbind SUPER, Space".to_owned()));
        assert!(log.contains(&"keyword unbind SUPER SHIFT, L".to_owned()));
    }

    #[test]
    fn an_unreachable_compositor_fails_with_a_snippet_to_copy() {
        let mut event_loop: EventLoop<'static, ()> = EventLoop::try_new().unwrap();
        let config = ShortcutsConfig {
            shortcuts: sample(),
            launcher: launcher(),
            kind: CompositorKind::Sway,
            strategy: Strategy::Sway("/nonexistent/sway.sock".into()),
            bus: BusKind::Address(String::new()),
        };
        let shortcuts = Shortcuts::start(&event_loop.handle(), config, |_| {}).unwrap();
        let state = shortcuts.state.clone();
        assert!(pump_until(&mut event_loop, || state.registration.get_untracked() != Registration::Pending));
        assert!(matches!(state.registration.get_untracked(), Registration::Failed(_)));
        assert!(state.snippet.get_untracked().contains("bindsym Mod4+Shift+l exec app action lock"));
    }

    #[test]
    fn sway_bindings_go_through_ipc() {
        let dir = tempfile::tempdir().unwrap();
        let socket = dir.path().join("sway.sock");
        serve_fake_sway(socket.clone());
        let mut event_loop: EventLoop<'static, ()> = EventLoop::try_new().unwrap();
        let config = ShortcutsConfig {
            shortcuts: sample(),
            launcher: launcher(),
            kind: CompositorKind::Sway,
            strategy: Strategy::Sway(socket),
            bus: BusKind::Address(String::new()),
        };
        let shortcuts = Shortcuts::start(&event_loop.handle(), config, |_| {}).unwrap();
        let state = shortcuts.state.clone();
        assert!(pump_until(&mut event_loop, || state.registration.get_untracked() != Registration::Pending));
        assert_eq!(state.registration.get_untracked(), Registration::Bound(Method::Sway));
    }

    const SERVICE: &str = "org.freedesktop.portal.Desktop";
    const PATH: &str = "/org/freedesktop/portal/desktop";

    struct FakePortal {
        bound: mpsc::Sender<Vec<(String, String)>>,
        session: String,
    }

    fn token_of(options: &HashMap<String, OwnedVariant>, key: &str) -> String {
        options.get(key).and_then(|v| String::try_from(v.try_clone().ok()?).ok()).unwrap_or_default()
    }

    #[interface(name = "org.freedesktop.portal.GlobalShortcuts")]
    impl FakePortal {
        #[zbus(property, name = "version")]
        fn version(&self) -> u32 {
            1
        }

        async fn create_session(
            &self,
            #[zbus(header)] header: zbus::message::Header<'_>,
            #[zbus(connection)] connection: &zbus::Connection,
            options: HashMap<String, OwnedVariant>,
        ) -> zbus::fdo::Result<OwnedObjectPath> {
            let sender = header.sender().map(|s| s.to_string()).unwrap_or_default();
            let path = crate::platform::dbus::portal_request_path(&sender, &token_of(&options, "handle_token"));
            let results = HashMap::from([(
                "session_handle".to_owned(),
                OwnedVariant::try_from(Variant::from(self.session.clone())).unwrap(),
            )]);
            connection
                .emit_signal(
                    None::<&str>,
                    path.as_str(),
                    "org.freedesktop.portal.Request",
                    "Response",
                    &(0u32, results),
                )
                .await?;
            Ok(OwnedObjectPath::try_from(path).unwrap())
        }

        async fn bind_shortcuts(
            &self,
            #[zbus(header)] header: zbus::message::Header<'_>,
            #[zbus(connection)] connection: &zbus::Connection,
            session: OwnedObjectPath,
            shortcuts: Vec<(String, HashMap<String, OwnedVariant>)>,
            parent: String,
            options: HashMap<String, OwnedVariant>,
        ) -> zbus::fdo::Result<OwnedObjectPath> {
            let _ = (session, parent);
            let seen = shortcuts.iter().map(|(id, o)| (id.clone(), token_of(o, "preferred_trigger"))).collect();
            let _ = self.bound.send(seen);
            let sender = header.sender().map(|s| s.to_string()).unwrap_or_default();
            let path = crate::platform::dbus::portal_request_path(&sender, &token_of(&options, "handle_token"));
            let none: HashMap<String, OwnedVariant> = HashMap::new();
            connection
                .emit_signal(None::<&str>, path.as_str(), "org.freedesktop.portal.Request", "Response", &(0u32, none))
                .await?;
            Ok(OwnedObjectPath::try_from(path).unwrap())
        }

        #[zbus(signal)]
        async fn activated(
            emitter: &SignalEmitter<'_>,
            session: OwnedObjectPath,
            id: &str,
            timestamp: u64,
            options: HashMap<String, OwnedVariant>,
        ) -> zbus::Result<()>;
    }

    #[test]
    fn the_portal_binds_triggers_and_reports_activations() {
        let bus = TestBus::start();
        let (bound_tx, bound_rx) = mpsc::channel();
        let (ready_tx, ready_rx) = mpsc::channel();
        let (fire_tx, fire_rx) = mpsc::channel::<()>();
        let address = bus.address.clone();
        std::thread::spawn(move || {
            async_io::block_on(async move {
                let session = "/org/freedesktop/portal/desktop/session/1_1/test".to_owned();
                let connection = zbus::connection::Builder::address(address.as_str())
                    .name(SERVICE)
                    .serve_at(PATH, FakePortal { bound: bound_tx, session: session.clone() })
                    .build()
                    .await
                    .unwrap();
                ready_tx.send(()).unwrap();
                let emitter = SignalEmitter::new(&connection, PATH).unwrap();
                loop {
                    if fire_rx.try_recv().is_ok() {
                        let path = OwnedObjectPath::try_from(session.clone()).unwrap();
                        FakePortal::activated(&emitter, path, "lock", 1, HashMap::new()).await.unwrap();
                    }
                    async_io::Timer::after(Duration::from_millis(20)).await;
                }
            });
        });
        ready_rx.recv_timeout(Duration::from_secs(10)).unwrap();

        let mut event_loop: EventLoop<'static, ()> = EventLoop::try_new().unwrap();
        let received = std::rc::Rc::new(std::cell::RefCell::new(Vec::<String>::new()));
        let sink = received.clone();
        let config = ShortcutsConfig {
            shortcuts: sample(),
            launcher: launcher(),
            kind: CompositorKind::Niri,
            strategy: Strategy::Portal,
            bus: bus.kind(),
        };
        let shortcuts =
            Shortcuts::start(&event_loop.handle(), config, move |id| sink.borrow_mut().push(id.to_owned())).unwrap();
        let state = shortcuts.state.clone();
        assert!(pump_until(&mut event_loop, || state.registration.get_untracked() != Registration::Pending));
        assert_eq!(state.registration.get_untracked(), Registration::Bound(Method::Portal));
        let seen = bound_rx.recv_timeout(Duration::from_secs(5)).unwrap();
        assert_eq!(
            seen,
            [("toggle-launcher".to_owned(), "LOGO+Space".to_owned()), ("lock".to_owned(), "LOGO+SHIFT+l".to_owned())]
        );

        fire_tx.send(()).unwrap();
        assert!(pump_until(&mut event_loop, || !received.borrow().is_empty()));
        assert_eq!(received.borrow().as_slice(), ["lock"]);
    }

    #[test]
    fn without_the_portal_the_state_is_manual_with_a_snippet() {
        let bus = TestBus::start();
        let mut event_loop: EventLoop<'static, ()> = EventLoop::try_new().unwrap();
        let config = ShortcutsConfig {
            shortcuts: sample(),
            launcher: launcher(),
            kind: CompositorKind::Niri,
            strategy: Strategy::Portal,
            bus: bus.kind(),
        };
        let shortcuts = Shortcuts::start(&event_loop.handle(), config, |_| {}).unwrap();
        let state = shortcuts.state.clone();
        assert!(pump_until(&mut event_loop, || state.registration.get_untracked() != Registration::Pending));
        assert_eq!(state.registration.get_untracked(), Registration::Manual);
        assert!(state.snippet.get_untracked().starts_with("binds {"));
    }
}
