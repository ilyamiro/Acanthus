use std::collections::HashMap;
use std::time::Instant;

use acanthus_core::{Signal, batch};
use acanthus_io::IoError;
use calloop::LoopHandle;
use futures_lite::StreamExt;
use serde_json::Value;
use zbus::{Connection, Message};
use zbus::{OwnedVariant as OwnedValue, Variant as ZValue};

use crate::platform::bridge::{EventSink, ServiceHandle, spawn_supervised};
use crate::platform::dbus::BusKind;

const PREFIX: &str = "org.mpris.MediaPlayer2.";
const OBJECT_PATH: &str = "/org/mpris/MediaPlayer2";
const PLAYER_IFACE: &str = "org.mpris.MediaPlayer2.Player";
const ROOT_IFACE: &str = "org.mpris.MediaPlayer2";

/// Playback state of a media player.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Playback {
    /// Not playing.
    #[default]
    Stopped,
    /// Playing.
    Playing,
    /// Paused.
    Paused,
}

impl Playback {
    /// The state for an MPRIS `PlaybackStatus` string; unknown values are stopped.
    pub fn parse(text: &str) -> Playback {
        match text {
            "Playing" => Playback::Playing,
            "Paused" => Playback::Paused,
            _ => Playback::Stopped,
        }
    }
}

/// A media player on the session bus and what it reports.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Player {
    /// Its D-Bus name.
    pub bus_name: String,
    /// Name of the player for display.
    pub identity: String,
    /// Name of the player's desktop entry.
    pub desktop_entry: String,
    /// Playback state.
    pub playback: Playback,
    /// Track title.
    pub title: String,
    /// Track artist.
    pub artist: String,
    /// Track album.
    pub album: String,
    /// URL of the cover art.
    pub art_url: String,
    /// Identifier of the track.
    pub track_id: String,
    /// Track length in microseconds, zero when unknown.
    pub length_us: i64,
    /// Position in microseconds when `position_stamp` was taken.
    pub position_us: i64,
    /// When the position was read.
    pub position_stamp: Option<Instant>,
    /// Playback rate.
    pub rate: f64,
    /// Volume from 0 to 1.
    pub volume: f64,
    /// True when shuffle is on.
    pub shuffle: bool,
    /// Loop status: `None`, `Track` or `Playlist`.
    pub loop_status: String,
    /// True when the player can be controlled.
    pub can_control: bool,
    /// True when it can start playing.
    pub can_play: bool,
    /// True when it can pause.
    pub can_pause: bool,
    /// True when it can skip to the next track.
    pub can_go_next: bool,
    /// True when it can skip to the previous track.
    pub can_go_previous: bool,
    /// True when it can seek.
    pub can_seek: bool,
    /// The complete track metadata as JSON.
    pub metadata: Value,
}

impl Player {
    /// True while playing.
    pub fn is_playing(&self) -> bool {
        self.playback == Playback::Playing
    }

    /// The position in microseconds extrapolated to `now`, bounded by the track length.
    pub fn live_position_us(&self, now: Instant) -> i64 {
        let elapsed = match (self.is_playing(), self.position_stamp) {
            (true, Some(stamp)) => {
                (now.saturating_duration_since(stamp).as_micros() as f64 * self.rate.max(0.0)) as i64
            }
            _ => 0,
        };
        let position = self.position_us + elapsed;
        if self.length_us > 0 { position.min(self.length_us) } else { position.max(0) }
    }

    /// The identifier of the player: its bus name, else its identity.
    pub fn id(&self) -> &str {
        if self.bus_name.is_empty() { &self.identity } else { &self.bus_name }
    }

    /// The cover art URL: the reported one, the one in the metadata, or the track URL when it points to an image.
    pub fn cover_url(&self) -> String {
        if !self.art_url.is_empty() {
            return self.art_url.clone();
        }
        let text = |key: &str| self.metadata.get(key).and_then(Value::as_str).unwrap_or("");
        for key in ["mpris:artUrl", "artUrl"] {
            if !text(key).is_empty() {
                return text(key).to_owned();
            }
        }
        let url = text("xesam:url");
        let lower = url.to_lowercase();
        let remote = url.starts_with("http") || url.starts_with("file://");
        let image = [".jpg", ".jpeg", ".png", ".webp"].iter().any(|ext| lower.ends_with(ext));
        if remote && image { url.to_owned() } else { String::new() }
    }
}

/// The player to show: the one playing, else the last that played, else a paused one with a title, else a controllable one, else the first by name.
pub fn select_active<'a>(players: &'a [Player], last_playing_bus: &str) -> Option<&'a Player> {
    let mut sorted: Vec<&Player> = players.iter().collect();
    sorted.sort_by(|a, b| a.bus_name.cmp(&b.bus_name));
    sorted
        .iter()
        .find(|p| p.is_playing())
        .or_else(|| {
            (!last_playing_bus.is_empty()).then(|| sorted.iter().find(|p| p.bus_name == last_playing_bus)).flatten()
        })
        .or_else(|| sorted.iter().find(|p| p.playback != Playback::Stopped && !p.title.is_empty()))
        .or_else(|| sorted.iter().find(|p| p.can_control))
        .or_else(|| sorted.first())
        .copied()
}

/// A command for a media player.
#[derive(Debug, Clone, PartialEq)]
pub enum Action {
    /// Start playing.
    Play,
    /// Pause.
    Pause,
    /// Toggle between playing and paused.
    PlayPause,
    /// Stop.
    Stop,
    /// Next track.
    Next,
    /// Previous track.
    Previous,
    /// Seek by this many microseconds.
    SeekBy(i64),
    /// Jump to this position in microseconds.
    SetPosition(i64),
    /// Ask the player to show its window.
    Raise,
}

/// An action addressed to a player.
#[derive(Debug, Clone, PartialEq)]
pub struct Command {
    /// D-Bus name of the player.
    pub bus_name: String,
    /// What to do.
    pub action: Action,
}

enum Event {
    Updated(Box<Player>),
    Removed(String),
}

fn as_str(value: &OwnedValue) -> Option<String> {
    match &**value {
        ZValue::Str(s) => Some(s.to_string()),
        ZValue::ObjectPath(p) => Some(p.to_string()),
        _ => None,
    }
}

fn as_i64(value: &OwnedValue) -> Option<i64> {
    match &**value {
        ZValue::I64(n) => Some(*n),
        ZValue::U64(n) => Some(*n as i64),
        ZValue::I32(n) => Some(i64::from(*n)),
        ZValue::U32(n) => Some(i64::from(*n)),
        _ => None,
    }
}

fn as_f64(value: &OwnedValue) -> Option<f64> {
    match &**value {
        ZValue::F64(n) => Some(*n),
        _ => None,
    }
}

fn as_bool(value: &OwnedValue) -> Option<bool> {
    match &**value {
        ZValue::Bool(b) => Some(*b),
        _ => None,
    }
}

fn metadata_json(value: &OwnedValue) -> Value {
    crate::platform::dbus::variant_to_json(value)
}

/// Builds a [`Player`] from the properties of the root and player interfaces, with the position stamped `now`.
pub fn parse_player(
    bus_name: &str,
    root: &HashMap<String, OwnedValue>,
    props: &HashMap<String, OwnedValue>,
    now: Instant,
) -> Player {
    let get = |key: &str| props.get(key);
    let flag = |key: &str| get(key).and_then(as_bool).unwrap_or(false);
    let mut player = Player {
        bus_name: bus_name.to_owned(),
        identity: root.get("Identity").and_then(as_str).unwrap_or_default(),
        desktop_entry: root.get("DesktopEntry").and_then(as_str).unwrap_or_default(),
        playback: Playback::parse(&get("PlaybackStatus").and_then(as_str).unwrap_or_default()),
        position_us: get("Position").and_then(as_i64).unwrap_or(0),
        position_stamp: Some(now),
        rate: get("Rate").and_then(as_f64).unwrap_or(1.0),
        volume: get("Volume").and_then(as_f64).unwrap_or(1.0),
        shuffle: flag("Shuffle"),
        loop_status: get("LoopStatus").and_then(as_str).unwrap_or_default(),
        can_control: flag("CanControl"),
        can_play: flag("CanPlay"),
        can_pause: flag("CanPause"),
        can_go_next: flag("CanGoNext"),
        can_go_previous: flag("CanGoPrevious"),
        can_seek: flag("CanSeek"),
        ..Default::default()
    };
    if let Some(metadata) = get("Metadata") {
        let json = metadata_json(metadata);
        let text = |key: &str| json.get(key).and_then(Value::as_str).unwrap_or("").to_owned();
        player.title = text("xesam:title");
        player.album = text("xesam:album");
        player.art_url = text("mpris:artUrl");
        player.track_id = text("mpris:trackid");
        player.artist = match json.get("xesam:artist") {
            Some(Value::Array(artists)) => artists.iter().filter_map(Value::as_str).collect::<Vec<_>>().join(", "),
            Some(Value::String(artist)) => artist.clone(),
            _ => String::new(),
        };
        player.length_us = json.get("mpris:length").and_then(Value::as_i64).unwrap_or(0);
        player.metadata = json;
    }
    player
}

async fn read_player(connection: &Connection, bus_name: &str) -> zbus::Result<Player> {
    let properties =
        zbus::Proxy::new(connection, bus_name.to_owned(), OBJECT_PATH, "org.freedesktop.DBus.Properties").await?;
    let root: HashMap<String, OwnedValue> = properties.call("GetAll", &(ROOT_IFACE,)).await?;
    let props: HashMap<String, OwnedValue> = properties.call("GetAll", &(PLAYER_IFACE,)).await?;
    Ok(parse_player(bus_name, &root, &props, Instant::now()))
}

async fn run_command(connection: &Connection, command: Command) {
    let result: zbus::Result<()> = async {
        let iface = if command.action == Action::Raise { ROOT_IFACE } else { PLAYER_IFACE };
        let proxy = zbus::Proxy::new(connection, command.bus_name.clone(), OBJECT_PATH, iface).await?;
        match &command.action {
            Action::Play => proxy.call::<_, _, ()>("Play", &()).await?,
            Action::Pause => proxy.call::<_, _, ()>("Pause", &()).await?,
            Action::PlayPause => proxy.call::<_, _, ()>("PlayPause", &()).await?,
            Action::Stop => proxy.call::<_, _, ()>("Stop", &()).await?,
            Action::Next => proxy.call::<_, _, ()>("Next", &()).await?,
            Action::Previous => proxy.call::<_, _, ()>("Previous", &()).await?,
            Action::SeekBy(offset) => proxy.call::<_, _, ()>("Seek", &(*offset,)).await?,
            Action::SetPosition(position) => {
                let props = zbus::Proxy::new(
                    connection,
                    command.bus_name.clone(),
                    OBJECT_PATH,
                    "org.freedesktop.DBus.Properties",
                )
                .await?;
                let metadata: HashMap<String, OwnedValue> = props
                    .call("Get", &(PLAYER_IFACE, "Metadata"))
                    .await
                    .map(|v: OwnedValue| v.try_into().unwrap_or_default())?;
                let track = metadata
                    .get("mpris:trackid")
                    .and_then(as_str)
                    .and_then(|t| zbus::ObjectPath::try_from(t).ok().map(|p| p.into_owned()))
                    .unwrap_or_else(|| {
                        zbus::ObjectPath::from_static_str_unchecked("/org/mpris/MediaPlayer2/TrackList/NoTrack")
                    });
                proxy.call::<_, _, ()>("SetPosition", &(track, *position)).await?
            }
            Action::Raise => proxy.call::<_, _, ()>("Raise", &()).await?,
        }
        Ok(())
    }
    .await;
    if let Err(e) = result {
        tracing::debug!("mpris command {:?} failed: {e}", command);
    }
}

enum Item {
    Names(Message),
    Props(Message),
    Seeked(Message),
    Command(Command),
}

async fn run(bus: BusKind, events: EventSink<Event>, commands: async_channel::Receiver<Command>) {
    let result: zbus::Result<()> = async {
        let connection = bus.connect_shared().await?;
        let dbus = zbus::fdo::DBusProxy::new(&connection).await?;

        let names_rule = zbus::MatchRule::builder()
            .msg_type(zbus::message::Type::Signal)
            .sender("org.freedesktop.DBus")
            .interface("org.freedesktop.DBus")
            .member("NameOwnerChanged")
            .build()?;
        let props_rule = zbus::MatchRule::builder()
            .msg_type(zbus::message::Type::Signal)
            .interface("org.freedesktop.DBus.Properties")
            .member("PropertiesChanged")
            .path(OBJECT_PATH)
            .build()?;
        let seeked_rule = zbus::MatchRule::builder()
            .msg_type(zbus::message::Type::Signal)
            .interface(PLAYER_IFACE)
            .member("Seeked")
            .path(OBJECT_PATH)
            .build()?;
        let names =
            zbus::MessageStream::for_match_rule(names_rule, &connection, None).await?.map(|m| m.map(Item::Names));
        let props =
            zbus::MessageStream::for_match_rule(props_rule, &connection, None).await?.map(|m| m.map(Item::Props));
        let seeked =
            zbus::MessageStream::for_match_rule(seeked_rule, &connection, None).await?.map(|m| m.map(Item::Seeked));
        let cmds = commands.map(|c| Ok(Item::Command(c)));
        let mut stream = std::pin::pin!(names.or(props).or(seeked).or(cmds));

        let mut owners: HashMap<String, String> = HashMap::new();
        for name in dbus.list_names().await? {
            let name = name.to_string();
            if name.starts_with(PREFIX) {
                if let Ok(owner) = dbus.get_name_owner(name.as_str().try_into()?).await {
                    owners.insert(owner.to_string(), name.clone());
                }
                if let Ok(player) = read_player(&connection, &name).await {
                    events.send(Event::Updated(Box::new(player)));
                }
            }
        }

        while let Some(item) = stream.next().await {
            let Ok(item) = item else { continue };
            match item {
                Item::Names(message) => {
                    let Ok((name, old, new)) = message.body().deserialize::<(String, String, String)>() else {
                        continue;
                    };
                    if !name.starts_with(PREFIX) {
                        continue;
                    }
                    owners.retain(|_, bus| *bus != name);
                    if new.is_empty() {
                        events.send(Event::Removed(name));
                    } else {
                        let _ = old;
                        owners.insert(new, name.clone());
                        if let Ok(player) = read_player(&connection, &name).await {
                            events.send(Event::Updated(Box::new(player)));
                        }
                    }
                }
                Item::Props(message) | Item::Seeked(message) => {
                    let Some(sender) = message.header().sender().map(|s| s.to_string()) else { continue };
                    let Some(bus_name) = owners.get(&sender).cloned() else { continue };
                    if let Ok(player) = read_player(&connection, &bus_name).await {
                        events.send(Event::Updated(Box::new(player)));
                    }
                }
                Item::Command(command) => run_command(&connection, command).await,
            }
        }
        Ok(())
    }
    .await;
    if let Err(e) = result {
        tracing::info!("mpris unavailable: {e}");
    }
}

/// Media players on the session bus as signals.
pub struct Mpris {
    /// All players.
    pub players: Signal<Vec<Player>>,
    /// The player to show, chosen by [`select_active`].
    pub active: Signal<Option<Player>>,
    service: ServiceHandle<Command>,
}

impl Mpris {
    /// Connects to the session bus on a helper thread.
    pub fn start<D: 'static>(handle: &LoopHandle<'static, D>, bus: BusKind) -> Result<Mpris, IoError> {
        let players: Signal<Vec<Player>> = Signal::new(Vec::new());
        let active: Signal<Option<Player>> = Signal::new(None);
        let (target_players, target_active) = (players.clone(), active.clone());
        let mut last_playing = String::new();
        let service = spawn_supervised(
            handle,
            "acanthus-mpris",
            move |event| {
                target_players.update(|list| match event {
                    Event::Updated(player) => match list.iter_mut().find(|p| p.bus_name == player.bus_name) {
                        Some(existing) => *existing = *player,
                        None => list.push(*player),
                    },
                    Event::Removed(bus_name) => list.retain(|p| p.bus_name != bus_name),
                });
                let list = target_players.get_untracked();
                let next = select_active(&list, &last_playing).cloned();
                if let Some(player) = next.as_ref().filter(|p| p.is_playing()) {
                    last_playing = player.bus_name.clone();
                }
                batch(|| target_active.set(next));
            },
            move |events, commands| run(bus.clone(), events, commands),
        )?;
        Ok(Mpris { players, active, service })
    }

    /// Sends `action` to the player with this bus name.
    pub fn send(&self, bus_name: &str, action: Action) {
        self.service.send(Command { bus_name: bus_name.to_owned(), action });
    }

    /// Toggles playback of the active player.
    pub fn play_pause_active(&self) {
        if let Some(player) = self.active.get_untracked() {
            self.send(&player.bus_name, Action::PlayPause);
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::mpsc;
    use std::time::Duration;

    use calloop::EventLoop;
    use serde_json::json;
    use zbus::interface;

    use super::*;
    use crate::platform::dbus::testbus::TestBus;

    fn player(bus: &str, playback: Playback, title: &str, can_control: bool) -> Player {
        Player { bus_name: bus.into(), playback, title: title.into(), can_control, ..Default::default() }
    }

    #[test]
    fn active_player_prefers_playing_then_last_then_loaded_then_controllable() {
        let players = vec![
            player("org.mpris.MediaPlayer2.b", Playback::Paused, "song", true),
            player("org.mpris.MediaPlayer2.a", Playback::Stopped, "", true),
            player("org.mpris.MediaPlayer2.c", Playback::Playing, "now", true),
        ];
        assert_eq!(select_active(&players, "").unwrap().bus_name, "org.mpris.MediaPlayer2.c");

        let paused = vec![players[0].clone(), players[1].clone()];
        assert_eq!(select_active(&paused, "org.mpris.MediaPlayer2.a").unwrap().bus_name, "org.mpris.MediaPlayer2.a");
        assert_eq!(select_active(&paused, "").unwrap().bus_name, "org.mpris.MediaPlayer2.b");

        let idle = vec![player("z", Playback::Stopped, "", false), player("y", Playback::Stopped, "", true)];
        assert_eq!(select_active(&idle, "").unwrap().bus_name, "y");
        let none_controllable =
            vec![player("z", Playback::Stopped, "", false), player("a", Playback::Stopped, "", false)];
        assert_eq!(select_active(&none_controllable, "").unwrap().bus_name, "a");
        assert!(select_active(&[], "").is_none());
    }

    #[test]
    fn position_interpolates_while_playing_and_clamps_to_length() {
        let start = Instant::now();
        let mut p = Player {
            playback: Playback::Playing,
            position_us: 1_000_000,
            position_stamp: Some(start),
            rate: 1.0,
            length_us: 3_000_000,
            ..Default::default()
        };
        assert_eq!(p.live_position_us(start + Duration::from_secs(1)), 2_000_000);
        assert_eq!(p.live_position_us(start + Duration::from_secs(10)), 3_000_000);
        p.rate = 2.0;
        assert_eq!(p.live_position_us(start + Duration::from_millis(500)), 2_000_000);
        p.playback = Playback::Paused;
        assert_eq!(p.live_position_us(start + Duration::from_secs(1)), 1_000_000);
    }

    #[test]
    fn cover_url_falls_back_through_metadata() {
        let mut p = Player::default();
        assert_eq!(p.cover_url(), "");
        p.metadata = json!({"xesam:url": "https://example.com/cover.PNG"});
        assert_eq!(p.cover_url(), "https://example.com/cover.PNG");
        p.metadata = json!({"xesam:url": "https://example.com/video"});
        assert_eq!(p.cover_url(), "");
        p.metadata = json!({"mpris:artUrl": "file:///a.jpg", "xesam:url": "x"});
        assert_eq!(p.cover_url(), "file:///a.jpg");
        p.art_url = "http://direct".into();
        assert_eq!(p.cover_url(), "http://direct");
    }

    struct FakeRoot;

    #[interface(name = "org.mpris.MediaPlayer2")]
    impl FakeRoot {
        #[zbus(property)]
        fn identity(&self) -> String {
            "Fake Player".into()
        }
        #[zbus(property)]
        fn desktop_entry(&self) -> String {
            "fake".into()
        }
        fn raise(&self) {}
    }

    struct FakePlayer {
        status: String,
        title: String,
        calls: mpsc::Sender<String>,
    }

    #[interface(name = "org.mpris.MediaPlayer2.Player")]
    impl FakePlayer {
        #[zbus(property)]
        fn playback_status(&self) -> String {
            self.status.clone()
        }
        #[zbus(property)]
        fn metadata(&self) -> HashMap<String, OwnedValue> {
            let mut map = HashMap::new();
            map.insert("xesam:title".to_owned(), OwnedValue::try_from(ZValue::from(self.title.as_str())).unwrap());
            map.insert(
                "xesam:artist".to_owned(),
                OwnedValue::try_from(ZValue::from(vec!["A".to_owned(), "B".to_owned()])).unwrap(),
            );
            map.insert("mpris:length".to_owned(), OwnedValue::try_from(ZValue::from(240_000_000i64)).unwrap());
            map.insert("mpris:artUrl".to_owned(), OwnedValue::try_from(ZValue::from("file:///cover.png")).unwrap());
            map
        }
        #[zbus(property)]
        fn position(&self) -> i64 {
            5_000_000
        }
        #[zbus(property)]
        fn rate(&self) -> f64 {
            1.0
        }
        #[zbus(property)]
        fn can_control(&self) -> bool {
            true
        }
        #[zbus(property)]
        fn can_go_next(&self) -> bool {
            true
        }
        fn play_pause(&self) {
            self.calls.send("PlayPause".into()).unwrap();
        }
        fn next(&self) {
            self.calls.send("Next".into()).unwrap();
        }
        fn seek(&self, offset: i64) {
            self.calls.send(format!("Seek {offset}")).unwrap();
        }
    }

    enum Change {
        Status(&'static str),
        Quit,
    }

    fn serve(address: String, calls: mpsc::Sender<String>, changes: mpsc::Receiver<Change>, ready: mpsc::Sender<()>) {
        std::thread::spawn(move || {
            async_io::block_on(async move {
                let connection = zbus::connection::Builder::address(address.as_str())
                    .name("org.mpris.MediaPlayer2.fake")
                    .serve_at(OBJECT_PATH, FakeRoot)
                    .serve_at(OBJECT_PATH, FakePlayer { status: "Paused".into(), title: "Song".into(), calls })
                    .build()
                    .await
                    .unwrap();
                ready.send(()).unwrap();
                while let Ok(change) = changes.recv() {
                    match change {
                        Change::Status(status) => {
                            let iface =
                                connection.object_server().interface::<_, FakePlayer>(OBJECT_PATH).await.unwrap();
                            iface.get_mut().await.status = status.into();
                            let emitter = iface.signal_emitter();
                            iface.get().await.playback_status_changed(emitter).await.unwrap();
                        }
                        Change::Quit => {
                            drop(connection);
                            return;
                        }
                    }
                }
            });
        });
    }

    fn pump_until(event_loop: &mut EventLoop<'static, ()>, done: impl Fn() -> bool) -> bool {
        let deadline = std::time::Instant::now() + Duration::from_secs(10);
        while !done() && std::time::Instant::now() < deadline {
            event_loop.dispatch(Duration::from_millis(10), &mut ()).unwrap();
        }
        done()
    }

    #[test]
    fn discovers_updates_controls_and_forgets_players() {
        let bus = TestBus::start();
        let (calls_tx, calls_rx) = mpsc::channel();
        let (change_tx, change_rx) = mpsc::channel();
        let (ready_tx, ready_rx) = mpsc::channel();
        serve(bus.address.clone(), calls_tx, change_rx, ready_tx);
        ready_rx.recv_timeout(Duration::from_secs(10)).unwrap();

        let mut event_loop: EventLoop<'static, ()> = EventLoop::try_new().unwrap();
        let mpris = Mpris::start(&event_loop.handle(), bus.kind()).unwrap();
        let active = mpris.active.clone();
        assert!(pump_until(&mut event_loop, || active.get_untracked().is_some()));
        let player = active.get_untracked().unwrap();
        assert_eq!(player.bus_name, "org.mpris.MediaPlayer2.fake");
        assert_eq!(
            (player.identity.as_str(), player.title.as_str(), player.artist.as_str()),
            ("Fake Player", "Song", "A, B")
        );
        assert_eq!((player.length_us, player.position_us, player.playback), (240_000_000, 5_000_000, Playback::Paused));
        assert!(player.can_control && player.can_go_next && !player.can_seek);
        assert_eq!(player.cover_url(), "file:///cover.png");

        change_tx.send(Change::Status("Playing")).unwrap();
        assert!(pump_until(&mut event_loop, || active.get_untracked().is_some_and(|p| p.is_playing())));

        mpris.play_pause_active();
        mpris.send("org.mpris.MediaPlayer2.fake", Action::Next);
        mpris.send("org.mpris.MediaPlayer2.fake", Action::SeekBy(-5_000_000));
        let mut seen = Vec::new();
        let deadline = std::time::Instant::now() + Duration::from_secs(10);
        while seen.len() < 3 && std::time::Instant::now() < deadline {
            event_loop.dispatch(Duration::from_millis(10), &mut ()).unwrap();
            while let Ok(call) = calls_rx.try_recv() {
                seen.push(call);
            }
        }
        assert_eq!(seen, ["PlayPause", "Next", "Seek -5000000"]);

        change_tx.send(Change::Quit).unwrap();
        assert!(pump_until(&mut event_loop, || mpris.players.get_untracked().is_empty()));
        assert!(active.get_untracked().is_none());
    }
}
