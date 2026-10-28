use std::rc::Rc;

use acanthus_core::{Signal, Vec2};

use crate::node::NodeId;

/// What a drop does with the data, as in `Qt.DropAction`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DropAction {
    /// The receiver keeps a copy.
    Copy,
    /// The data moves to the receiver and the source removes its own.
    Move,
    /// The receiver links to the data.
    Link,
}

/// A set of [`DropAction`]s.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DropActions {
    /// Copy is allowed.
    pub copy: bool,
    /// Move is allowed.
    pub move_: bool,
    /// Link is allowed.
    pub link: bool,
}

impl DropActions {
    /// Every action.
    pub const ALL: DropActions = DropActions { copy: true, move_: true, link: true };
    /// Copy only.
    pub const COPY: DropActions = DropActions { copy: true, move_: false, link: false };
    /// Move only.
    pub const MOVE: DropActions = DropActions { copy: false, move_: true, link: false };

    /// True when `action` is in the set.
    pub fn contains(self, action: DropAction) -> bool {
        match action {
            DropAction::Copy => self.copy,
            DropAction::Move => self.move_,
            DropAction::Link => self.link,
        }
    }

    /// The action a drop between a source offering `self` and a target accepting `accepted` performs: copy, then move, then link; `None` when they share none.
    pub fn negotiate(self, accepted: DropActions) -> Option<DropAction> {
        [DropAction::Copy, DropAction::Move, DropAction::Link]
            .into_iter()
            .find(|action| self.contains(*action) && accepted.contains(*action))
    }
}

impl Default for DropActions {
    fn default() -> Self {
        DropActions::ALL
    }
}

/// The payload of a drag: byte blobs by MIME type, like `QMimeData`.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct DragData {
    entries: Vec<(String, Rc<[u8]>)>,
}

impl DragData {
    /// An empty payload.
    pub fn new() -> DragData {
        DragData::default()
    }

    /// Adds or replaces the data for `mime`.
    pub fn with(mut self, mime: &str, bytes: impl Into<Rc<[u8]>>) -> DragData {
        let bytes = bytes.into();
        match self.entries.iter_mut().find(|(m, _)| m == mime) {
            Some(entry) => entry.1 = bytes,
            None => self.entries.push((mime.to_owned(), bytes)),
        }
        self
    }

    /// A payload carrying `text` as `text/plain;charset=utf-8`.
    pub fn text(text: &str) -> DragData {
        DragData::new().with("text/plain;charset=utf-8", text.as_bytes())
    }

    /// A payload carrying a `text/uri-list` of `uris`.
    pub fn uri_list<S: AsRef<str>>(uris: &[S]) -> DragData {
        let body: String = uris.iter().map(|u| format!("{}\r\n", u.as_ref())).collect();
        DragData::new().with("text/uri-list", body.as_bytes())
    }

    /// The MIME types present, in insertion order.
    pub fn mime_types(&self) -> Vec<String> {
        self.entries.iter().map(|(m, _)| m.clone()).collect()
    }

    /// The bytes for `mime`.
    pub fn get(&self, mime: &str) -> Option<&[u8]> {
        self.entries.iter().find(|(m, _)| m == mime).map(|(_, bytes)| &bytes[..])
    }

    /// The text of the first text type present.
    pub fn as_text(&self) -> Option<String> {
        ["text/plain;charset=utf-8", "text/plain", "UTF8_STRING", "STRING"]
            .into_iter()
            .find_map(|mime| self.get(mime))
            .map(|bytes| String::from_utf8_lossy(bytes).into_owned())
    }

    /// The URIs of the `text/uri-list` entry, comments and blank lines skipped.
    pub fn uris(&self) -> Vec<String> {
        self.get("text/uri-list")
            .map(|bytes| {
                String::from_utf8_lossy(bytes)
                    .lines()
                    .map(str::trim)
                    .filter(|line| !line.is_empty() && !line.starts_with('#'))
                    .map(str::to_owned)
                    .collect()
            })
            .unwrap_or_default()
    }

    /// True when there is no data.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

/// What a drag in progress offers a [`DropArea`] before any data moves.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DragOffer {
    /// MIME types the source can provide.
    pub mime_types: Vec<String>,
    /// Actions the source allows.
    pub actions: DropActions,
    /// The node the drag started from, for drags inside the scene.
    pub source: Option<NodeId>,
}

/// A drop delivered to a [`DropArea`].
#[derive(Debug, Clone)]
pub struct DropEvent {
    /// The drop position in the area's own coordinates.
    pub position: Vec2,
    /// The dropped data.
    pub data: Rc<DragData>,
    /// The action chosen from what the source allows and the area accepts.
    pub action: DropAction,
    /// The node the drag started from, for drags inside the scene.
    pub source: Option<NodeId>,
}

type EnteredHandler = Box<dyn FnMut(&DragOffer)>;

/// A region that receives drags, as in QML's `DropArea`. It accepts a drag when one of its `keys` is among the offered MIME types (any drag when it has no keys) and an action is shared.
pub struct DropArea {
    /// True while a drag it accepts is over it.
    pub contains_drag: Signal<bool>,
    /// The position of the drag over it, in its own coordinates.
    pub drag_position: Signal<Vec2>,
    pub(crate) keys: Vec<String>,
    pub(crate) accepted: DropActions,
    pub(crate) on_entered: Option<EnteredHandler>,
    pub(crate) on_exited: Option<Box<dyn FnMut()>>,
    pub(crate) on_moved: Option<Box<dyn FnMut(Vec2)>>,
    pub(crate) on_dropped: Option<Box<dyn FnMut(DropEvent)>>,
}

impl Default for DropArea {
    fn default() -> Self {
        DropArea {
            contains_drag: Signal::new(false),
            drag_position: Signal::new(Vec2::ZERO),
            keys: Vec::new(),
            accepted: DropActions::ALL,
            on_entered: None,
            on_exited: None,
            on_moved: None,
            on_dropped: None,
        }
    }
}

impl DropArea {
    /// An area accepting every drag.
    pub fn new() -> DropArea {
        DropArea::default()
    }

    /// Restricts the area to drags offering one of these MIME types.
    pub fn keys<S: AsRef<str>>(mut self, keys: &[S]) -> DropArea {
        self.keys = keys.iter().map(|k| k.as_ref().to_owned()).collect();
        self
    }

    /// Restricts the actions the area performs.
    pub fn actions(mut self, actions: DropActions) -> DropArea {
        self.accepted = actions;
        self
    }

    /// Sets the callback run when an accepted drag enters.
    pub fn on_entered(mut self, f: impl FnMut(&DragOffer) + 'static) -> DropArea {
        self.on_entered = Some(Box::new(f));
        self
    }

    /// Sets the callback run when the drag leaves or is cancelled.
    pub fn on_exited(mut self, f: impl FnMut() + 'static) -> DropArea {
        self.on_exited = Some(Box::new(f));
        self
    }

    /// Sets the callback run when the drag moves inside the area.
    pub fn on_position_changed(mut self, f: impl FnMut(Vec2) + 'static) -> DropArea {
        self.on_moved = Some(Box::new(f));
        self
    }

    /// Sets the callback run when the drag is dropped on the area.
    pub fn on_dropped(mut self, f: impl FnMut(DropEvent) + 'static) -> DropArea {
        self.on_dropped = Some(Box::new(f));
        self
    }

    pub(crate) fn accepts(&self, offer: &DragOffer) -> Option<DropAction> {
        let keyed = self.keys.is_empty() || self.keys.iter().any(|k| offer.mime_types.contains(k));
        if keyed { offer.actions.negotiate(self.accepted) } else { None }
    }
}

type FinishedHandler = Box<dyn FnMut(Option<DropAction>)>;

/// Makes a node the start of drags inside the scene, as `DragHandler` with `Drag.mimeData` does in QML: pressing it and moving past `threshold` starts a drag carrying the data `data` returns.
pub struct DragSource {
    /// True while a drag started from this node is running.
    pub active: Signal<bool>,
    pub(crate) data: Box<dyn FnMut() -> DragData>,
    pub(crate) actions: DropActions,
    pub(crate) threshold: f32,
    pub(crate) on_finished: Option<FinishedHandler>,
}

impl DragSource {
    /// A source whose drags carry what `data` builds at the moment the drag starts.
    pub fn new(data: impl FnMut() -> DragData + 'static) -> DragSource {
        DragSource {
            active: Signal::new(false),
            data: Box::new(data),
            actions: DropActions::ALL,
            threshold: 10.0,
            on_finished: None,
        }
    }

    /// Sets the actions the source allows.
    pub fn actions(mut self, actions: DropActions) -> DragSource {
        self.actions = actions;
        self
    }

    /// Sets the distance in logical pixels the pointer moves before the drag starts (default 10, Qt's `startDragDistance`).
    pub fn threshold(mut self, threshold: f32) -> DragSource {
        self.threshold = threshold.max(0.0);
        self
    }

    /// Sets the callback run when the drag ends, with the action performed or `None` when it was cancelled or dropped nowhere.
    pub fn on_finished(mut self, f: impl FnMut(Option<DropAction>) + 'static) -> DragSource {
        self.on_finished = Some(Box::new(f));
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn actions_negotiate_copy_then_move_then_link() {
        assert_eq!(DropActions::ALL.negotiate(DropActions::ALL), Some(DropAction::Copy));
        assert_eq!(DropActions::ALL.negotiate(DropActions::MOVE), Some(DropAction::Move));
        assert_eq!(DropActions::COPY.negotiate(DropActions::MOVE), None);
        let link = DropActions { copy: false, move_: false, link: true };
        assert_eq!(DropActions::ALL.negotiate(link), Some(DropAction::Link));
    }

    #[test]
    fn data_round_trips_text_and_uri_lists() {
        let data = DragData::text("hello").with("application/x-id", &b"42"[..]);
        assert_eq!(data.as_text().as_deref(), Some("hello"));
        assert_eq!(data.get("application/x-id"), Some(&b"42"[..]));
        assert_eq!(data.mime_types(), ["text/plain;charset=utf-8", "application/x-id"]);
        let replaced = data.with("application/x-id", &b"7"[..]);
        assert_eq!(replaced.get("application/x-id"), Some(&b"7"[..]));
        assert_eq!(replaced.mime_types().len(), 2);

        let uris = DragData::uri_list(&["file:///a.png", "file:///b c.png"]);
        assert_eq!(uris.uris(), ["file:///a.png", "file:///b c.png"]);
        let commented = DragData::new().with("text/uri-list", &b"# note\r\n\r\nfile:///x\r\n"[..]);
        assert_eq!(commented.uris(), ["file:///x"]);
        assert!(DragData::new().is_empty());
    }

    #[test]
    fn an_area_filters_by_key_and_action() {
        let offer = DragOffer { mime_types: vec!["text/uri-list".into()], actions: DropActions::COPY, source: None };
        assert_eq!(DropArea::new().accepts(&offer), Some(DropAction::Copy));
        assert_eq!(DropArea::new().keys(&["text/uri-list"]).accepts(&offer), Some(DropAction::Copy));
        assert_eq!(DropArea::new().keys(&["image/png"]).accepts(&offer), None);
        assert_eq!(DropArea::new().actions(DropActions::MOVE).accepts(&offer), None);
    }
}
