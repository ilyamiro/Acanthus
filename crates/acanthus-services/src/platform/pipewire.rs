use std::cell::RefCell;
use std::collections::{BTreeMap, HashMap};
use std::io::Cursor;
use std::rc::Rc;

use acanthus_core::{Signal, batch, untrack};
use acanthus_io::IoError;
use calloop::LoopHandle;
use pipewire as pw;
use pw::metadata::{Metadata, MetadataListener};
use pw::node::{Node, NodeListener};
use pw::spa::param::ParamType;
use pw::spa::pod::deserialize::PodDeserializer;
use pw::spa::pod::serialize::PodSerializer;
use pw::spa::pod::{Object, Pod, Property, Value, ValueArray};
use pw::spa::utils::SpaTypes;
use pw::types::ObjectType;

use crate::platform::bridge::{EventSink, ThreadHandle, spawn_thread};

const PROP_MUTE: u32 = 65540;
const PROP_CHANNEL_VOLUMES: u32 = 65544;
const KEY_DEFAULT_SINK: &str = "default.audio.sink";
const KEY_DEFAULT_SOURCE: &str = "default.audio.source";
const DEFAULT_CHANNELS: usize = 2;

/// An audio node of PipeWire: a device or a stream.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct PwNode {
    /// PipeWire's node id.
    pub id: u32,
    /// Node name.
    pub name: String,
    /// Human-readable description.
    pub description: String,
    /// Short name.
    pub nick: String,
    /// The media class such as `Audio/Sink` or `Stream/Output/Audio`.
    pub media_class: String,
    /// All properties the node carries.
    pub properties: BTreeMap<String, String>,
    /// True for application streams.
    pub is_stream: bool,
    /// True for outputs (sinks and playback streams).
    pub is_sink: bool,
    /// True when the node carries audio.
    pub has_audio: bool,
    /// True when muted.
    pub muted: bool,
    /// Volume per channel, linear.
    pub volumes: Vec<f32>,
}

impl PwNode {
    /// The mean volume over the channels, zero when unknown.
    pub fn volume(&self) -> f32 {
        if self.volumes.is_empty() { 0.0 } else { self.volumes.iter().sum::<f32>() / self.volumes.len() as f32 }
    }

    /// The property with this key.
    pub fn property(&self, key: &str) -> Option<&str> {
        self.properties.get(key).map(String::as_str)
    }

    /// The best name to show: the device description, the node description or the node name.
    pub fn display_name(&self) -> &str {
        [self.property("device.description").unwrap_or(""), &self.description, &self.name]
            .into_iter()
            .find(|s| !s.is_empty())
            .unwrap_or("Unknown Device")
    }
}

/// The audio nodes and defaults at one moment.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct PipewireSnapshot {
    /// All audio nodes.
    pub nodes: Vec<PwNode>,
    /// Name of the default sink.
    pub default_sink: Option<String>,
    /// Name of the default source.
    pub default_source: Option<String>,
}

/// A request to change audio settings.
#[derive(Debug, Clone, PartialEq)]
pub enum PipewireCommand {
    /// Sets the volume of the node with this id, linear.
    SetVolume(u32, f32),
    /// Mutes or unmutes the node with this id.
    SetMuted(u32, bool),
    /// Makes the sink with this name the default.
    SetDefaultSink(String),
    /// Makes the source with this name the default.
    SetDefaultSource(String),
}

/// The audio node for the registry object `id` with `properties`, or `None` when it is not an audio node.
pub fn node_from_properties(id: u32, properties: &BTreeMap<String, String>) -> Option<PwNode> {
    let media_class = properties.get("media.class")?.clone();
    if !media_class.contains("Audio") {
        return None;
    }
    let text = |key: &str| properties.get(key).cloned().unwrap_or_default();
    Some(PwNode {
        id,
        name: text("node.name"),
        description: text("node.description"),
        nick: text("node.nick"),
        is_stream: media_class.starts_with("Stream/"),
        is_sink: media_class.contains("Sink"),
        media_class,
        properties: properties.clone(),
        ..PwNode::default()
    })
}

/// The channel volumes and mute flag in a props parameter, each when present.
pub fn parse_props(bytes: &[u8]) -> Option<(Option<Vec<f32>>, Option<bool>)> {
    let (_, value) = PodDeserializer::deserialize_any_from(bytes).ok()?;
    let Value::Object(object) = value else {
        return None;
    };
    let mut volumes = None;
    let mut muted = None;
    for property in object.properties {
        match (property.key, property.value) {
            (PROP_CHANNEL_VOLUMES, Value::ValueArray(ValueArray::Float(values))) => volumes = Some(values),
            (PROP_MUTE, Value::Bool(flag)) => muted = Some(flag),
            _ => {}
        }
    }
    Some((volumes, muted))
}

fn props_bytes(properties: Vec<Property>) -> Option<Vec<u8>> {
    let object =
        Value::Object(Object { type_: SpaTypes::ObjectParamProps.as_raw(), id: ParamType::Props.as_raw(), properties });
    PodSerializer::serialize(Cursor::new(Vec::new()), &object).ok().map(|(cursor, _)| cursor.into_inner())
}

/// A props parameter that sets all `channels` to `volume`.
pub fn volume_props(channels: usize, volume: f32) -> Option<Vec<u8>> {
    let volume = if volume.is_finite() { volume.max(0.0) } else { 0.0 };
    props_bytes(vec![Property::new(
        PROP_CHANNEL_VOLUMES,
        Value::ValueArray(ValueArray::Float(vec![volume; channels.max(1)])),
    )])
}

/// A props parameter that sets the mute flag.
pub fn mute_props(muted: bool) -> Option<Vec<u8>> {
    props_bytes(vec![Property::new(PROP_MUTE, Value::Bool(muted))])
}

/// The JSON PipeWire stores for a default device with this name.
pub fn default_node_json(name: &str) -> String {
    serde_json::json!({ "name": name }).to_string()
}

/// The device name in the JSON PipeWire stores for a default device.
pub fn parse_default_node(value: &str) -> Option<String> {
    let parsed: serde_json::Value = serde_json::from_str(value).ok()?;
    parsed.get("name")?.as_str().map(str::to_owned)
}

/// The output devices among `nodes`.
pub fn outputs(nodes: &[PwNode]) -> Vec<PwNode> {
    nodes.iter().filter(|n| !n.is_stream && n.is_sink && n.has_audio).cloned().collect()
}

/// The input devices among `nodes`, without monitors.
pub fn inputs(nodes: &[PwNode]) -> Vec<PwNode> {
    nodes
        .iter()
        .filter(|n| {
            !n.is_stream
                && !n.is_sink
                && n.has_audio
                && n.property("device.class") != Some("monitor")
                && !n.name.ends_with(".monitor")
        })
        .cloned()
        .collect()
}

/// The application playback and capture streams among `nodes`.
pub fn apps(nodes: &[PwNode]) -> Vec<PwNode> {
    nodes
        .iter()
        .filter(|n| n.is_stream && n.has_audio && n.property("application.id") != Some("org.PulseAudio.pavucontrol"))
        .cloned()
        .collect()
}

/// True when some audible playback stream is running.
pub fn has_active_stream(nodes: &[PwNode]) -> bool {
    nodes.iter().any(|n| {
        n.has_audio
            && n.media_class == "Stream/Output/Audio"
            && n.property("pulse.corked") != Some("true")
            && !n.muted
            && n.volume() > 0.001
    })
}

enum Event {
    Snapshot(Box<PipewireSnapshot>),
    Unavailable,
}

/// Observable state of the audio service.
#[derive(Clone)]
pub struct PipewireState {
    /// True once the first snapshot arrived.
    pub ready: Signal<bool>,
    /// True when PipeWire can be reached.
    pub available: Signal<bool>,
    /// All audio nodes.
    pub nodes: Signal<Vec<PwNode>>,
    /// Name of the default sink.
    pub default_sink: Signal<Option<String>>,
    /// Name of the default source.
    pub default_source: Signal<Option<String>>,
    /// Name of the device the virtual sink forwards to.
    pub route: Signal<String>,
    /// Name of the application's virtual sink.
    pub virtual_sink: String,
}

/// The sink audio really goes to: the route when the default sink is the virtual sink and a route is set, else the default sink.
pub fn effective_sink(default_sink: Option<&str>, route: &str, virtual_sink: &str) -> Option<String> {
    match default_sink {
        Some(name) if name == virtual_sink && !route.is_empty() => Some(route.to_owned()),
        Some(name) => Some(name.to_owned()),
        None => None,
    }
}

impl PipewireState {
    fn new(virtual_sink: &str) -> Self {
        PipewireState {
            ready: Signal::new(false),
            available: Signal::new(false),
            nodes: Signal::new(Vec::new()),
            default_sink: Signal::new(None),
            default_source: Signal::new(None),
            route: Signal::new(String::new()),
            virtual_sink: virtual_sink.to_owned(),
        }
    }

    /// [`effective_sink`] for the current state.
    pub fn effective_default_sink(&self) -> Option<String> {
        effective_sink(self.default_sink.get().as_deref(), &self.route.get(), &self.virtual_sink)
    }

    /// Copies a snapshot into the signals in one batch.
    pub fn apply(&self, snapshot: &PipewireSnapshot) {
        batch(|| {
            self.available.set(true);
            if self.nodes.with_untracked(|nodes| *nodes != snapshot.nodes) {
                self.nodes.set(snapshot.nodes.clone());
            }
            self.default_sink.set(snapshot.default_sink.clone());
            self.default_source.set(snapshot.default_source.clone());
            self.ready.set(true);
        });
    }

    fn lost(&self) {
        batch(|| {
            self.available.set(false);
            self.nodes.set(Vec::new());
            self.default_sink.set(None);
            self.default_source.set(None);
            self.ready.set(true);
        });
    }

    /// The node with this id.
    pub fn node(&self, id: u32) -> Option<PwNode> {
        self.nodes.with_untracked(|nodes| nodes.iter().find(|n| n.id == id).cloned())
    }

    /// The node with this name.
    pub fn node_by_name(&self, name: &str) -> Option<PwNode> {
        self.nodes.with_untracked(|nodes| nodes.iter().find(|n| n.name == name).cloned())
    }

    /// The node of the default sink.
    pub fn default_sink_node(&self) -> Option<PwNode> {
        let name = untrack(|| self.effective_default_sink());
        name.and_then(|name| self.node_by_name(&name))
    }

    /// The node of the default source.
    pub fn default_source_node(&self) -> Option<PwNode> {
        self.default_source.with_untracked(|name| name.as_deref().and_then(|name| self.node_by_name(name)))
    }
}

/// Audio control through PipeWire: devices, streams, volumes and defaults, as signals.
pub struct Pipewire {
    /// Its observable state.
    pub state: PipewireState,
    commands: pw::channel::Sender<Message>,
    _thread: ThreadHandle,
}

enum Message {
    Command(PipewireCommand),
    Quit,
}

impl Pipewire {
    /// Connects to PipeWire on a helper thread; `virtual_sink` names the application's own sink, if it has one.
    pub fn start<D: 'static>(handle: &LoopHandle<'static, D>, virtual_sink: &str) -> Result<Pipewire, IoError> {
        let state = PipewireState::new(virtual_sink);
        let target = state.clone();
        let (commands, receiver) = pw::channel::channel::<Message>();
        let thread = spawn_thread(
            handle,
            "acanthus-pipewire",
            move |event| match event {
                Event::Snapshot(snapshot) => target.apply(&snapshot),
                Event::Unavailable => target.lost(),
            },
            move |events, _stop| {
                if run(&events, receiver).is_err() {
                    events.send(Event::Unavailable);
                }
            },
        )?;
        Ok(Pipewire { state, commands, _thread: thread })
    }

    /// Sends a command; false when the service is gone.
    pub fn command(&self, command: PipewireCommand) -> bool {
        self.commands.send(Message::Command(command)).is_ok()
    }

    /// Sets a node's volume, linear.
    pub fn set_volume(&self, id: u32, volume: f32) -> bool {
        self.command(PipewireCommand::SetVolume(id, volume))
    }

    /// Mutes or unmutes a node.
    pub fn set_muted(&self, id: u32, muted: bool) -> bool {
        self.command(PipewireCommand::SetMuted(id, muted))
    }

    /// Toggles the mute flag of a node; false when the node is unknown.
    pub fn toggle_mute(&self, id: u32) -> bool {
        match self.state.node(id) {
            Some(node) => self.set_muted(id, !node.muted),
            None => false,
        }
    }

    /// Makes the sink the default.
    pub fn set_default_sink(&self, name: &str) -> bool {
        self.command(PipewireCommand::SetDefaultSink(name.to_owned()))
    }

    /// Makes the source the default.
    pub fn set_default_source(&self, name: &str) -> bool {
        self.command(PipewireCommand::SetDefaultSource(name.to_owned()))
    }
}

impl Drop for Pipewire {
    fn drop(&mut self) {
        let _ = self.commands.send(Message::Quit);
    }
}

struct Entry {
    node: PwNode,
    proxy: Node,
    _listener: NodeListener,
}

struct Model {
    events: EventSink<Event>,
    entries: HashMap<u32, Entry>,
    metadata: Option<(Metadata, MetadataListener)>,
    default_sink: Option<String>,
    default_source: Option<String>,
    last: Option<PipewireSnapshot>,
}

impl Model {
    fn publish(&mut self) {
        let mut nodes: Vec<PwNode> = self.entries.values().map(|e| e.node.clone()).collect();
        nodes.sort_by_key(|n| n.id);
        let snapshot = PipewireSnapshot {
            nodes,
            default_sink: self.default_sink.clone(),
            default_source: self.default_source.clone(),
        };
        if self.last.as_ref() != Some(&snapshot) {
            self.last = Some(snapshot.clone());
            self.events.send(Event::Snapshot(Box::new(snapshot)));
        }
    }
}

fn dict_to_map(dict: &pw::spa::utils::dict::DictRef) -> BTreeMap<String, String> {
    dict.iter().map(|(k, v)| (k.to_owned(), v.to_owned())).collect()
}

fn run(events: &EventSink<Event>, receiver: pw::channel::Receiver<Message>) -> Result<(), pw::Error> {
    pw::init();
    let main_loop = pw::main_loop::MainLoopRc::new(None)?;
    let context = pw::context::ContextRc::new(&main_loop, None)?;
    let core = context.connect_rc(None)?;
    let registry = core.get_registry_rc()?;

    let lost = Rc::new(RefCell::new(false));
    let loop_weak = main_loop.downgrade();
    let lost_flag = lost.clone();
    let _core_listener = core
        .add_listener_local()
        .error(move |id, _seq, _res, _message| {
            if id == 0 {
                *lost_flag.borrow_mut() = true;
                if let Some(main_loop) = loop_weak.upgrade() {
                    main_loop.quit();
                }
            }
        })
        .register();

    let model = Rc::new(RefCell::new(Model {
        events: events.clone(),
        entries: HashMap::new(),
        metadata: None,
        default_sink: None,
        default_source: None,
        last: None,
    }));

    let registry_weak = registry.downgrade();
    let added = model.clone();
    let removed = model.clone();
    let _registry_listener = registry
        .add_listener_local()
        .global(move |global| {
            let Some(registry) = registry_weak.upgrade() else {
                return;
            };
            let Some(props) = global.props else {
                return;
            };
            match global.type_ {
                ObjectType::Node => {
                    let properties = dict_to_map(props);
                    let Some(node) = node_from_properties(global.id, &properties) else {
                        return;
                    };
                    let Ok(proxy) = registry.bind::<Node, _>(global) else {
                        return;
                    };
                    proxy.subscribe_params(&[ParamType::Props]);
                    let id = global.id;
                    let info_model = added.clone();
                    let param_model = added.clone();
                    let listener = proxy
                        .add_listener_local()
                        .info(move |info| {
                            let Some(props) = info.props() else {
                                return;
                            };
                            let properties = dict_to_map(props);
                            let mut model = info_model.borrow_mut();
                            let Some(entry) = model.entries.get_mut(&id) else {
                                return;
                            };
                            if let Some(fresh) = node_from_properties(id, &properties) {
                                entry.node = PwNode {
                                    has_audio: entry.node.has_audio,
                                    muted: entry.node.muted,
                                    volumes: entry.node.volumes.clone(),
                                    ..fresh
                                };
                            }
                            model.publish();
                        })
                        .param(move |_seq, kind, _index, _next, pod| {
                            if kind != ParamType::Props {
                                return;
                            }
                            let Some((volumes, muted)) = pod.and_then(|p| parse_props(p.as_bytes())) else {
                                return;
                            };
                            let mut model = param_model.borrow_mut();
                            let Some(entry) = model.entries.get_mut(&id) else {
                                return;
                            };
                            if let Some(volumes) = volumes {
                                entry.node.volumes = volumes;
                                entry.node.has_audio = true;
                            }
                            if let Some(muted) = muted {
                                entry.node.muted = muted;
                            }
                            model.publish();
                        })
                        .register();
                    let mut model = added.borrow_mut();
                    model.entries.insert(id, Entry { node, proxy, _listener: listener });
                    model.publish();
                }
                ObjectType::Metadata => {
                    if props.get("metadata.name") != Some("default") {
                        return;
                    }
                    let Ok(metadata) = registry.bind::<Metadata, _>(global) else {
                        return;
                    };
                    let target = added.clone();
                    let listener = metadata
                        .add_listener_local()
                        .property(move |subject, key, _type, value| {
                            if subject != 0 {
                                return 0;
                            }
                            let name = value.and_then(parse_default_node);
                            let mut model = target.borrow_mut();
                            match key {
                                Some(KEY_DEFAULT_SINK) => model.default_sink = name,
                                Some(KEY_DEFAULT_SOURCE) => model.default_source = name,
                                _ => return 0,
                            }
                            model.publish();
                            0
                        })
                        .register();
                    added.borrow_mut().metadata = Some((metadata, listener));
                }
                _ => {}
            }
        })
        .global_remove(move |id| {
            let mut model = removed.borrow_mut();
            if model.entries.remove(&id).is_some() {
                model.publish();
            }
        })
        .register();

    let command_model = model.clone();
    let loop_weak = main_loop.downgrade();
    let _receiver = receiver.attach(main_loop.loop_(), move |message| match message {
        Message::Quit => {
            if let Some(main_loop) = loop_weak.upgrade() {
                main_loop.quit();
            }
        }
        Message::Command(command) => apply_command(&command_model.borrow(), command),
    });

    main_loop.run();
    if *lost.borrow() {
        events.send(Event::Unavailable);
    }
    Ok(())
}

fn apply_command(model: &Model, command: PipewireCommand) {
    let set_default = |key: &str, name: &str| {
        if let Some((metadata, _)) = &model.metadata {
            metadata.set_property(0, key, Some("Spa:String:JSON"), Some(&default_node_json(name)));
        }
    };
    match command {
        PipewireCommand::SetVolume(id, volume) => {
            let Some(entry) = model.entries.get(&id) else {
                return;
            };
            let channels = if entry.node.volumes.is_empty() { DEFAULT_CHANNELS } else { entry.node.volumes.len() };
            set_props(entry, volume_props(channels, volume));
        }
        PipewireCommand::SetMuted(id, muted) => {
            if let Some(entry) = model.entries.get(&id) {
                set_props(entry, mute_props(muted));
            }
        }
        PipewireCommand::SetDefaultSink(name) => set_default(KEY_DEFAULT_SINK, &name),
        PipewireCommand::SetDefaultSource(name) => set_default(KEY_DEFAULT_SOURCE, &name),
    }
}

fn set_props(entry: &Entry, bytes: Option<Vec<u8>>) {
    if let Some(pod) = bytes.as_deref().and_then(Pod::from_bytes) {
        entry.proxy.set_param(ParamType::Props, 0, pod);
    }
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use calloop::EventLoop;

    use super::*;

    const VIRTUAL_SINK: &str = "demo_sink";

    fn props(entries: &[(&str, &str)]) -> BTreeMap<String, String> {
        entries.iter().map(|(k, v)| ((*k).to_owned(), (*v).to_owned())).collect()
    }

    #[test]
    fn the_routed_sink_stands_in_for_the_virtual_default() {
        assert_eq!(effective_sink(Some(VIRTUAL_SINK), "alsa_out", VIRTUAL_SINK).as_deref(), Some("alsa_out"));
        assert_eq!(effective_sink(Some(VIRTUAL_SINK), "", VIRTUAL_SINK).as_deref(), Some(VIRTUAL_SINK));
        assert_eq!(effective_sink(Some("speakers"), "alsa_out", VIRTUAL_SINK).as_deref(), Some("speakers"));
        assert_eq!(effective_sink(None, "alsa_out", VIRTUAL_SINK), None);
    }

    #[test]
    fn the_default_sink_node_follows_the_route() {
        let state = PipewireState::new(VIRTUAL_SINK);
        let nodes = vec![
            audio(1, "speakers", "Audio/Sink", vec![0.4, 0.4], false),
            audio(2, VIRTUAL_SINK, "Audio/Sink", vec![1.0, 1.0], false),
        ];
        state.apply(&PipewireSnapshot { nodes, default_sink: Some(VIRTUAL_SINK.to_owned()), default_source: None });
        assert_eq!(state.default_sink_node().map(|n| n.id), Some(2));
        state.route.set("speakers".to_owned());
        assert_eq!(state.default_sink_node().map(|n| n.id), Some(1));
    }

    fn audio(id: u32, name: &str, class: &str, volumes: Vec<f32>, muted: bool) -> PwNode {
        let mut node = node_from_properties(id, &props(&[("media.class", class), ("node.name", name)])).unwrap();
        node.has_audio = true;
        node.volumes = volumes;
        node.muted = muted;
        node
    }

    #[test]
    fn only_audio_nodes_are_tracked() {
        assert!(node_from_properties(1, &props(&[("media.class", "Video/Source")])).is_none());
        assert!(node_from_properties(1, &props(&[("node.name", "x")])).is_none());
        let sink = node_from_properties(
            2,
            &props(&[("media.class", "Audio/Sink"), ("node.name", "alsa_out"), ("node.description", "Speakers")]),
        )
        .unwrap();
        assert!(sink.is_sink && !sink.is_stream);
        assert_eq!(sink.display_name(), "Speakers");
        let stream = node_from_properties(3, &props(&[("media.class", "Stream/Output/Audio")])).unwrap();
        assert!(stream.is_stream && !stream.is_sink);
    }

    #[test]
    fn volume_is_the_channel_average() {
        let node = audio(1, "a", "Audio/Sink", vec![0.2, 0.6], false);
        assert!((node.volume() - 0.4).abs() < 1e-6);
        assert_eq!(PwNode::default().volume(), 0.0);
    }

    #[test]
    fn volume_pod_round_trips() {
        let bytes = volume_props(2, 0.75).unwrap();
        let (volumes, muted) = parse_props(&bytes).unwrap();
        assert_eq!(volumes, Some(vec![0.75, 0.75]));
        assert_eq!(muted, None);
        let (volumes, _) = parse_props(&volume_props(3, -1.0).unwrap()).unwrap();
        assert_eq!(volumes, Some(vec![0.0; 3]));
    }

    #[test]
    fn mute_pod_round_trips() {
        let (volumes, muted) = parse_props(&mute_props(true).unwrap()).unwrap();
        assert_eq!(volumes, None);
        assert_eq!(muted, Some(true));
    }

    #[test]
    fn default_node_metadata_round_trips() {
        let json = default_node_json("alsa_output.pci");
        assert_eq!(parse_default_node(&json).as_deref(), Some("alsa_output.pci"));
        assert_eq!(parse_default_node("not json"), None);
        assert_eq!(parse_default_node("{}"), None);
    }

    #[test]
    fn nodes_are_grouped_like_the_qml_filters() {
        let mut monitor = audio(3, "alsa_out.monitor", "Audio/Source", vec![1.0], false);
        monitor.properties.insert("device.class".into(), "monitor".into());
        let mut pavucontrol = audio(5, "pavu", "Stream/Output/Audio", vec![1.0], false);
        pavucontrol.properties.insert("application.id".into(), "org.PulseAudio.pavucontrol".into());
        let nodes = vec![
            audio(1, "out", "Audio/Sink", vec![0.5, 0.5], false),
            audio(2, "mic", "Audio/Source", vec![1.0], false),
            monitor,
            audio(4, "player", "Stream/Output/Audio", vec![1.0], false),
            pavucontrol,
        ];
        assert_eq!(outputs(&nodes).iter().map(|n| n.id).collect::<Vec<_>>(), [1]);
        assert_eq!(inputs(&nodes).iter().map(|n| n.id).collect::<Vec<_>>(), [2]);
        assert_eq!(apps(&nodes).iter().map(|n| n.id).collect::<Vec<_>>(), [4]);
    }

    #[test]
    fn active_stream_ignores_corked_muted_and_silent_streams() {
        let playing = audio(1, "a", "Stream/Output/Audio", vec![0.5], false);
        let muted = audio(2, "b", "Stream/Output/Audio", vec![0.5], true);
        let silent = audio(3, "c", "Stream/Output/Audio", vec![0.0], false);
        let mut corked = audio(4, "d", "Stream/Output/Audio", vec![0.5], false);
        corked.properties.insert("pulse.corked".into(), "true".into());
        assert!(!has_active_stream(&[muted, silent, corked]));
        assert!(has_active_stream(&[playing]));
    }

    #[test]
    fn state_resolves_default_nodes_by_name() {
        let state = PipewireState::new(VIRTUAL_SINK);
        state.apply(&PipewireSnapshot {
            nodes: vec![
                audio(1, "out", "Audio/Sink", vec![0.5], false),
                audio(2, "mic", "Audio/Source", vec![1.0], false),
            ],
            default_sink: Some("out".into()),
            default_source: Some("gone".into()),
        });
        assert_eq!(state.default_sink_node().map(|n| n.id), Some(1));
        assert!(state.default_source_node().is_none());
        assert!(state.ready.get_untracked() && state.available.get_untracked());
        state.lost();
        assert!(!state.available.get_untracked());
        assert!(state.nodes.with_untracked(Vec::is_empty));
    }

    #[test]
    fn live_daemon_is_listed_when_one_is_running() {
        let runtime = std::env::var_os("XDG_RUNTIME_DIR").map(std::path::PathBuf::from);
        let Some(socket) = runtime.map(|dir| dir.join("pipewire-0")) else {
            return;
        };
        if !socket.exists() {
            return;
        }
        let mut event_loop: EventLoop<'static, ()> = EventLoop::try_new().unwrap();
        let pipewire = Pipewire::start(&event_loop.handle(), VIRTUAL_SINK).unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        while pipewire.state.nodes.with_untracked(Vec::is_empty) && Instant::now() < deadline {
            event_loop.dispatch(Duration::from_millis(20), &mut ()).unwrap();
        }
        assert!(pipewire.state.available.get_untracked());
        assert!(pipewire.state.nodes.with_untracked(|nodes| nodes.iter().all(|n| n.media_class.contains("Audio"))));
    }
}
