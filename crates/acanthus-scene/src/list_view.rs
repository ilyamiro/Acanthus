use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};
use std::hash::Hash;
use std::rc::Rc;

use acanthus_anim::{AnimHandle, Animation, Easing, Parallel, Smoothed, Tween};
use acanthus_core::{Effect, ListSignal, Signal, Vec2, batch, untrack};

use crate::animate::Prop;
use crate::dynamic::{Host, Item};
use crate::flick::Flickable;
use crate::node::{Node, NodeId, Orientation};
use crate::scene::{FlickableNodes, Scene};

pub const DEFAULT_CACHE_BUFFER: f32 = 320.0;
const MEASURE_EPSILON: f32 = 0.01;
const UNMEASURED_EXTENT: f32 = 1.0;

/// Where [`ListView::position_view_at_index`] places an item, as in Qt's `positionViewAtIndex`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ViewPosition {
    /// The item's start at the viewport start.
    Beginning,
    /// The item centred in the viewport.
    Center,
    /// The item's end at the viewport end.
    End,
    /// Scrolls the minimum needed to make the item at least partly visible.
    Visible,
    /// Scrolls the minimum needed to show the item entirely.
    Contain,
}

/// One property animated by a view transition, a QML `NumberAnimation` inside a `Transition`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct NumberAnimation {
    /// The animated property: [`Prop::X`], [`Prop::Y`], [`Prop::Opacity`], [`Prop::Scale`] or [`Prop::Rotation`]; others are ignored.
    pub prop: Prop,
    /// Start value, or `None` for the item's value when the transition starts.
    pub from: Option<f32>,
    /// End value, or `None` for the end state: the item's destination for `x` and `y`, the value when the transition starts otherwise.
    pub to: Option<f32>,
    /// Duration in milliseconds (QML default 250).
    pub ms: u32,
    /// Easing curve (QML default linear).
    pub easing: Easing,
}

impl NumberAnimation {
    /// Animates `prop` over 250 ms, linear, from its current value to its end state, as a bare QML `NumberAnimation`.
    pub fn new(prop: Prop) -> Self {
        NumberAnimation { prop, from: None, to: None, ms: 250, easing: Easing::Linear }
    }

    /// Sets the start value.
    pub fn from(mut self, from: f32) -> Self {
        self.from = Some(from);
        self
    }

    /// Sets the end value.
    pub fn to(mut self, to: f32) -> Self {
        self.to = Some(to);
        self
    }

    /// Sets the duration in milliseconds and the easing.
    pub fn over(mut self, ms: u32, easing: Easing) -> Self {
        self.ms = ms;
        self.easing = easing;
        self
    }
}

/// The view transitions of a list, QML `add`, `remove` and `displaced`; an empty list means no transition (the item jumps, as in Qt).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ListTransitions {
    /// Run on an item added to the model, placed at its destination first.
    pub add: Vec<NumberAnimation>,
    /// Run on an item removed from the model, which is destroyed when it finishes.
    pub remove: Vec<NumberAnimation>,
    /// Run on items whose position changes because others were added, removed or moved, from the old position to the new one.
    pub displaced: Vec<NumberAnimation>,
}

/// Static options of a list view.
#[derive(Debug, Clone, PartialEq)]
pub struct ListViewConfig {
    /// Direction items are laid out along.
    pub orientation: Orientation,
    /// Gap between items.
    pub spacing: f32,
    /// Extent outside the viewport in which items stay instantiated.
    pub cache_buffer: f32,
    /// Add, remove and displaced transitions.
    pub transitions: ListTransitions,
    /// Speed of the highlight moving to a new current item in pixels per second, QML `highlightMoveVelocity` (400).
    pub highlight_move_velocity: f32,
    /// Longest highlight move in milliseconds, QML `highlightMoveDuration` (`None`, the velocity alone).
    pub highlight_move_ms: Option<u32>,
    /// Speed of the highlight resizing in pixels per second, QML `highlightResizeVelocity` (400).
    pub highlight_resize_velocity: f32,
    /// Longest highlight resize in milliseconds, QML `highlightResizeDuration` (`None`).
    pub highlight_resize_ms: Option<u32>,
    /// Cell size that turns the list into a QML `GridView`, or `None` for a list.
    pub cell: Option<Vec2>,
}

impl Default for ListViewConfig {
    fn default() -> Self {
        ListViewConfig {
            orientation: Orientation::Vertical,
            spacing: 0.0,
            cache_buffer: DEFAULT_CACHE_BUFFER,
            transitions: ListTransitions::default(),
            highlight_move_velocity: 400.0,
            highlight_move_ms: None,
            highlight_resize_velocity: 400.0,
            highlight_resize_ms: None,
            cell: None,
        }
    }
}

impl ListViewConfig {
    /// A default configuration laid out horizontally.
    pub fn horizontal() -> Self {
        ListViewConfig { orientation: Orientation::Horizontal, ..Default::default() }
    }

    /// A QML `GridView` of `cell_width` x `cell_height` cells: items fill a row left to right and the view scrolls vertically (`GridView.FlowLeftToRight`); with [`Orientation::Horizontal`] they fill a column top to bottom and the view scrolls horizontally (`FlowTopToBottom`). A line holds as many whole cells as fit the viewport, at least one; spacing is ignored, as Qt's grid has none. The highlight takes the current item's size at once and moves over 150 ms, as GridView's does.
    pub fn grid(orientation: Orientation, cell_width: f32, cell_height: f32) -> Self {
        ListViewConfig {
            orientation,
            cell: Some(Vec2::new(cell_width, cell_height)),
            highlight_move_velocity: -1.0,
            highlight_move_ms: Some(150),
            ..Default::default()
        }
    }

    /// Sets the gap between items.
    pub fn spacing(mut self, spacing: f32) -> Self {
        self.spacing = spacing;
        self
    }

    /// Sets the extent kept instantiated outside the viewport.
    pub fn cache_buffer(mut self, cache_buffer: f32) -> Self {
        self.cache_buffer = cache_buffer;
        self
    }

    /// Sets the add, remove and displaced transitions.
    pub fn transitions(mut self, transitions: ListTransitions) -> Self {
        self.transitions = transitions;
        self
    }
}

#[derive(Default)]
struct Metrics {
    starts: Vec<f32>,
    extents: Vec<f32>,
}

/// Handle to a virtualised list created by [`Scene::list_view`].
#[derive(Clone)]
pub struct ListView {
    /// The viewport and content nodes.
    pub nodes: FlickableNodes,
    /// The scrolling state.
    pub flickable: Flickable,
    /// The number of items in the model.
    pub count: Signal<usize>,
    /// The number of items currently instantiated.
    pub instantiated: Signal<usize>,
    /// The size of the viewport.
    pub view_size: Signal<Vec2>,
    /// Index of the current item, QML `currentIndex` (0 at the start); setting it scrolls the view to keep the item, or its highlight, in sight.
    pub current_index: Signal<usize>,
    orientation: Orientation,
    metrics: Rc<RefCell<Metrics>>,
    highlight: Signal<Option<NodeId>>,
    follow: Rc<Cell<bool>>,
}

impl ListView {
    /// Start and extent of the item at `index` along the layout axis, or `None` when it is out of range.
    pub fn item_range(&self, index: usize) -> Option<(f32, f32)> {
        let metrics = self.metrics.borrow();
        Some((*metrics.starts.get(index)?, *metrics.extents.get(index)?))
    }

    /// Adds `node` behind the items as the highlight of the current item, QML `highlight` with `highlightFollowsCurrentItem`: it takes the current item's size and moves and resizes to the next one with [`Smoothed`] at the configured velocities. Returns its id.
    pub fn set_highlight(&self, scene: &mut Scene, mut node: Node) -> NodeId {
        if let Some(old) = self.highlight.get_untracked() {
            scene.remove(old);
        }
        node.width.get_or_insert_with(|| Signal::new(0.0));
        node.height.get_or_insert_with(|| Signal::new(0.0));
        let vertical = self.orientation == Orientation::Vertical;
        let (pos, extent) = if vertical {
            (node.y.clone(), node.height.clone().expect("set above"))
        } else {
            (node.x.clone(), node.width.clone().expect("set above"))
        };
        let view = self.clone();
        let tracking = Effect::new(move || {
            let tracked = (pos.get(), extent.get());
            if !view.follow.get() {
                return;
            }
            if view.flickable.dragging.get_untracked() || view.flickable.flicking.get_untracked() {
                view.follow.set(false);
                return;
            }
            if let Some(item) = view.item_range(view.current_index.get_untracked()) {
                track(&view.flickable, view.view_size.get_untracked(), vertical, tracked, item);
            }
        });
        let id = scene.add(self.nodes.content, node.z(-1.0).bind(tracking));
        self.highlight.set(Some(id));
        id
    }

    /// Scrolls so that the item at `index` sits as `mode` describes; does nothing for an out-of-range index.
    pub fn position_view_at_index(&self, index: usize, mode: ViewPosition) {
        let Some((start, extent)) = self.item_range(index) else { return };
        let view = self.view_size.get_untracked();
        let vertical = self.orientation == Orientation::Vertical;
        let (viewport, current) = if vertical {
            (view.y, self.flickable.content_y.get_untracked())
        } else {
            (view.x, self.flickable.content_x.get_untracked())
        };
        let end = start + extent;
        let target = match mode {
            ViewPosition::Beginning => start,
            ViewPosition::End => end - viewport,
            ViewPosition::Center => start + extent / 2.0 - viewport / 2.0,
            ViewPosition::Visible | ViewPosition::Contain => {
                if start < current {
                    start
                } else if end > current + viewport {
                    end - viewport
                } else {
                    current
                }
            }
        };
        let position = self.flickable.position();
        if vertical {
            self.flickable.scroll_to(position.x, target, view);
        } else {
            self.flickable.scroll_to(target, position.y, view);
        }
    }
}

fn track(flickable: &Flickable, view: Vec2, vertical: bool, tracked: (f32, f32), item: (f32, f32)) {
    let ((tracked_pos, tracked_size), (item_pos, item_size)) = (tracked, item);
    let (size, view_pos) = if vertical {
        (view.y, flickable.content_y.get_untracked())
    } else {
        (view.x, flickable.content_x.get_untracked())
    };
    let (tracked_end, item_end) = (tracked_pos + tracked_size, item_pos + item_size);
    let mut pos = view_pos;
    if tracked_end >= view_pos + size && item_end >= view_pos + size {
        pos = if tracked_end <= item_end {
            if tracked_size > size { tracked_pos } else { tracked_end - size }
        } else if item_size > size {
            item_pos
        } else {
            item_end - size
        };
    }
    if tracked_pos < pos && item_pos < pos {
        pos = tracked_pos.max(item_pos);
    }
    if pos != view_pos {
        let position = flickable.position();
        if vertical {
            flickable.scroll_to(position.x, pos, view);
        } else {
            flickable.scroll_to(pos, position.y, view);
        }
    }
}

struct Highlight {
    node: NodeId,
    target: Option<[f32; 4]>,
    runs: [Option<AnimHandle>; 4],
    velocities: [Rc<Cell<f32>>; 4],
}

impl Highlight {
    fn new(node: NodeId) -> Self {
        Highlight { node, target: None, runs: [None; 4], velocities: Default::default() }
    }

    fn update(&mut self, scene: &Scene, target: [f32; 4], vertical: bool, config: &ListViewConfig) {
        let Some(node) = scene.get(self.node) else { return };
        let (width, height) = (node.width.clone(), node.height.clone());
        let (Some(width), Some(height)) = (width, height) else { return };
        let signals = if vertical {
            [node.y.clone(), height, width, node.x.clone()]
        } else {
            [node.x.clone(), width, height, node.y.clone()]
        };
        let previous = self.target.replace(target);
        for i in 0..signals.len() {
            if (i == 3 && config.cell.is_none()) || previous.is_some_and(|p| p[i] == target[i]) {
                continue;
            }
            if let Some(run) = self.runs[i].take() {
                run.stop();
            }
            if previous.is_none() || (config.cell.is_some() && (i == 1 || i == 2)) {
                signals[i].set(target[i]);
                continue;
            }
            let (velocity, ms) = if i == 0 || i == 3 {
                (config.highlight_move_velocity, config.highlight_move_ms)
            } else {
                (config.highlight_resize_velocity, config.highlight_resize_ms)
            };
            let run = Smoothed::new(&signals[i], target[i], velocity, ms).with_velocity(self.velocities[i].clone());
            self.runs[i] = Some(acanthus_anim::start(run));
        }
    }
}

impl Drop for Highlight {
    fn drop(&mut self) {
        for run in self.runs.iter_mut().filter_map(Option::take) {
            run.stop();
        }
    }
}

struct Entry<T> {
    node: NodeId,
    item: Item<T>,
    at: (f32, Option<f32>),
    run: Option<AnimHandle>,
}

fn run_transition(
    scene: &Scene,
    node: NodeId,
    anims: &[NumberAnimation],
    (along, cross): (f32, Option<f32>),
    vertical: bool,
) -> Option<Parallel> {
    let target = scene.get(node)?;
    let dest = if vertical {
        Vec2::new(cross.unwrap_or_else(|| target.x.get_untracked()), along)
    } else {
        Vec2::new(along, cross.unwrap_or_else(|| target.y.get_untracked()))
    };
    let mut group = None::<Parallel>;
    let mut moved = [false; 2];
    for anim in anims {
        let (signal, end) = match anim.prop {
            Prop::X => (&target.x, Some(dest.x)),
            Prop::Y => (&target.y, Some(dest.y)),
            Prop::Opacity => (&target.opacity, None),
            Prop::Scale => (&target.scale, None),
            Prop::Rotation => (&target.rotation, None),
            other => {
                tracing::warn!("view transition cannot animate {}", other.name());
                continue;
            }
        };
        moved[0] |= anim.prop == Prop::X;
        moved[1] |= anim.prop == Prop::Y;
        let current = signal.get_untracked();
        let to = anim.to.or(end).unwrap_or(current);
        let tween = Tween::new(signal, to, anim.ms, anim.easing).from(anim.from.unwrap_or(current));
        group = Some(group.unwrap_or_default().with(tween));
    }
    if !moved[0] {
        target.x.set(dest.x);
    }
    if !moved[1] {
        target.y.set(dest.y);
    }
    group
}

type Delegate<T> = Box<dyn FnMut(&mut Scene, NodeId, Item<T>) -> NodeId>;

struct Leaving {
    node: NodeId,
    done: Rc<Cell<bool>>,
}

struct Reaped {
    group: Parallel,
    done: Rc<Cell<bool>>,
    reap: Signal<u32>,
}

impl Animation for Reaped {
    fn start(&mut self) {
        self.group.start();
    }

    fn advance(&mut self, dt: f32) -> Option<f32> {
        let rest = self.group.advance(dt);
        if rest.is_some() {
            self.done.set(true);
            self.reap.update(|n| *n += 1);
        }
        rest
    }
}

struct ListHost<T, K> {
    viewport: NodeId,
    content: NodeId,
    flickable: Flickable,
    config: ListViewConfig,
    model: ListSignal<T>,
    key: Box<dyn Fn(&T) -> K>,
    delegate: Delegate<T>,
    entries: HashMap<K, Entry<T>>,
    extents: RefCell<HashMap<K, f32>>,
    metrics: Rc<RefCell<Metrics>>,
    count: Signal<usize>,
    instantiated: Signal<usize>,
    view_size: Signal<Vec2>,
    order: Vec<K>,
    primed: bool,
    leaving: Vec<Leaving>,
    reap: Signal<u32>,
    current_index: Signal<usize>,
    last_current: Option<usize>,
    follow: Rc<Cell<bool>>,
    highlight_node: Signal<Option<NodeId>>,
    highlight: Option<Highlight>,
}

impl<T, K> ListHost<T, K> {
    fn vertical(&self) -> bool {
        self.config.orientation == Orientation::Vertical
    }

    fn retire(&mut self, scene: &mut Scene, entry: Entry<T>) {
        if let Some(run) = entry.run {
            run.stop();
        }
        let node = entry.node;
        let vertical = self.vertical();
        let along =
            scene.get(node).map_or(entry.at.0, |n| if vertical { n.y.get_untracked() } else { n.x.get_untracked() });
        let Some(group) = run_transition(scene, node, &self.config.transitions.remove, (along, None), vertical) else {
            scene.remove(node);
            return;
        };
        let done = Rc::new(Cell::new(false));
        acanthus_anim::start(Reaped { group, done: done.clone(), reap: self.reap.clone() });
        self.leaving.push(Leaving { node, done });
    }
}

impl<T: Clone + PartialEq + 'static, K: Eq + Hash + Clone + 'static> Host for ListHost<T, K> {
    fn wants_measure(&self) -> bool {
        true
    }

    fn measure(&self, scene: &Scene) -> bool {
        let mut changed = false;
        if let Some(rect) = scene.geometry(self.viewport) {
            let size = Vec2::new(rect.w, rect.h);
            if self.view_size.get_untracked() != size {
                self.view_size.set(size);
                changed = true;
            }
        }
        let vertical = self.vertical();
        let mut extents = self.extents.borrow_mut();
        for (key, entry) in &self.entries {
            let Some(rect) = scene.geometry(entry.node) else { continue };
            let extent = if vertical { rect.h } else { rect.w };
            if extents.get(key).is_none_or(|old| (old - extent).abs() > MEASURE_EPSILON) {
                extents.insert(key.clone(), extent);
                changed = true;
            }
        }
        changed
    }

    fn sync(&mut self, scene: &mut Scene) -> bool {
        if !scene.contains(self.viewport) || !scene.contains(self.content) {
            for (_, entry) in self.entries.drain() {
                if let Some(run) = entry.run {
                    run.stop();
                }
                scene.remove(entry.node);
            }
            for gone in self.leaving.drain(..) {
                scene.remove(gone.node);
            }
            return false;
        }
        self.reap.get();
        let (finished, running): (Vec<Leaving>, Vec<Leaving>) =
            std::mem::take(&mut self.leaving).into_iter().partition(|l| l.done.get());
        self.leaving = running;
        for gone in finished {
            scene.remove(gone.node);
        }
        let values = self.model.get();
        let mut seen = HashSet::new();
        let keyed: Vec<(K, T)> =
            values.into_iter().map(|v| ((self.key)(&v), v)).filter(|(k, _)| seen.insert(k.clone())).collect();
        let stale: Vec<K> = self.entries.keys().filter(|k| !seen.contains(*k)).cloned().collect();
        for key in stale {
            if let Some(entry) = self.entries.remove(&key) {
                self.retire(scene, entry);
            }
        }
        self.extents.borrow_mut().retain(|k, _| seen.contains(k));
        let order: Vec<K> = keyed.iter().map(|(k, _)| k.clone()).collect();
        let added: HashSet<K> = if self.primed {
            let known: HashSet<&K> = self.order.iter().collect();
            order.iter().filter(|k| !known.contains(k)).cloned().collect()
        } else {
            HashSet::new()
        };
        let displacing = self.primed && self.order != order;
        self.order = order;
        self.primed = true;

        let vertical = self.vertical();
        let view = self.view_size.get();
        let grid = self.config.cell.map(|cell| {
            let (along, cross, room) = if vertical { (cell.y, cell.x, view.x) } else { (cell.x, cell.y, view.y) };
            (along, cross, ((room / cross.max(UNMEASURED_EXTENT)).floor() as usize).max(1))
        });
        let measured: Vec<f32> = {
            let extents = self.extents.borrow();
            keyed.iter().filter_map(|(k, _)| extents.get(k).copied()).collect()
        };
        let estimate = grid.map(|(along, _, _)| along).or_else(|| {
            (!measured.is_empty())
                .then(|| (measured.iter().sum::<f32>() / measured.len() as f32).max(UNMEASURED_EXTENT))
        });
        let spacing = self.config.spacing;
        let mut starts = Vec::with_capacity(keyed.len());
        let mut extents = Vec::with_capacity(keyed.len());
        let mut cursor = 0.0;
        if let Some((along, _, per)) = grid {
            starts.extend((0..keyed.len()).map(|i| (i / per) as f32 * along));
            extents.resize(keyed.len(), along);
            cursor = keyed.len().div_ceil(per) as f32 * along + spacing;
        } else {
            let known = self.extents.borrow();
            for (key, _) in &keyed {
                let extent = known.get(key).copied().or(estimate).unwrap_or(UNMEASURED_EXTENT);
                starts.push(cursor);
                extents.push(extent);
                cursor += extent + spacing;
            }
        }
        let total = if keyed.is_empty() { 0.0 } else { cursor - spacing };

        let scroll = if vertical { self.flickable.content_y.get() } else { self.flickable.content_x.get() };
        let viewport_extent = if vertical { view.y } else { view.x };
        let (first, last) = if keyed.is_empty() {
            (0, 0)
        } else if estimate.is_none() {
            (0, 1)
        } else {
            let low = scroll - self.config.cache_buffer;
            let high = scroll + viewport_extent + self.config.cache_buffer;
            let first = starts.iter().zip(&extents).position(|(s, e)| s + e > low).unwrap_or(keyed.len());
            let last = starts.partition_point(|s| *s < high);
            (first.min(keyed.len()), last.max(first).min(keyed.len()))
        };

        let current = self.current_index.get();
        let current_key = keyed.get(current).map(|(k, _)| k.clone());
        let wanted: HashSet<K> = keyed[first..last].iter().map(|(k, _)| k.clone()).collect();
        let out_of_range: Vec<K> = self.entries.keys().filter(|k| !wanted.contains(*k)).cloned().collect();
        for key in out_of_range {
            if let Some(entry) = self.entries.remove(&key) {
                if let Some(run) = entry.run {
                    run.stop();
                }
                scene.remove(entry.node);
            }
        }

        batch(|| {
            for (index, (key, value)) in keyed.into_iter().enumerate() {
                if index < first || index >= last {
                    continue;
                }
                let at = (starts[index], grid.map(|(_, cross, per)| (index % per) as f32 * cross));
                let transitions = &self.config.transitions;
                match self.entries.get_mut(&key) {
                    Some(entry) => {
                        entry.item.value.set(value);
                        entry.item.index.set(index);
                        if entry.at == at {
                            continue;
                        }
                        entry.at = at;
                        if let Some(run) = entry.run.take() {
                            run.stop();
                        }
                        let anims: &[NumberAnimation] = if displacing { &transitions.displaced } else { &[] };
                        entry.run = run_transition(scene, entry.node, anims, at, vertical).map(acanthus_anim::start);
                    }
                    None => {
                        let item = Item { value: Signal::new(value), index: Signal::new(index) };
                        let node = untrack(|| (self.delegate)(scene, self.content, item.clone()));
                        let anims: &[NumberAnimation] = if added.contains(&key) { &transitions.add } else { &[] };
                        let run = run_transition(scene, node, anims, at, vertical).map(acanthus_anim::start);
                        self.entries.insert(key, Entry { node, item, at, run });
                    }
                }
            }
            if vertical {
                self.flickable.content_height.set(total);
                self.flickable.content_width.set(view.x);
            } else {
                self.flickable.content_width.set(total);
                self.flickable.content_height.set(view.y);
            }
            self.count.set(starts.len());
            self.instantiated.set(self.entries.len());
        });
        let changed = self.last_current.replace(current) != Some(current);
        if changed {
            self.follow.set(true);
        }
        let highlight_node = self.highlight_node.get();
        if self.highlight.as_ref().map(|h| h.node) != highlight_node {
            self.highlight = highlight_node.map(Highlight::new);
        }
        let item = starts.get(current).map(|start| (*start, extents[current]));
        if let Some(highlight) = &mut self.highlight {
            if let Some(node) = scene.get(highlight.node) {
                node.visible.set(item.is_some());
            }
            if let Some((start, extent)) = item {
                let size = current_key.and_then(|k| self.entries.get(&k)).and_then(|e| scene.geometry(e.node));
                let size = size.map(|r| if vertical { (r.h, r.w) } else { (r.w, r.h) });
                let target = match grid {
                    Some((along, cross, per)) => {
                        let (a, c) = size.unwrap_or((along, cross));
                        [start, a, c, (current % per) as f32 * cross]
                    }
                    None => [start, extent, size.map_or(if vertical { view.x } else { view.y }, |s| s.1), 0.0],
                };
                highlight.update(scene, target, vertical, &self.config);
            }
        } else if changed
            && self.follow.get()
            && let Some(item) = item
        {
            track(&self.flickable, view, vertical, item, item);
        }
        {
            let mut metrics = self.metrics.borrow_mut();
            metrics.starts = starts;
            metrics.extents = extents;
        }
        true
    }
}

impl Scene {
    /// Creates a virtualised list under `parent` inside `viewport`, instantiating `delegate` only for items near the visible range of `model`; with [`ListViewConfig::grid`] it lays the items out as a QML `GridView`.
    pub fn list_view<T, K>(
        &mut self,
        parent: NodeId,
        viewport: Node,
        config: ListViewConfig,
        model: &ListSignal<T>,
        key: impl Fn(&T) -> K + 'static,
        delegate: impl FnMut(&mut Scene, NodeId, Item<T>) -> NodeId + 'static,
    ) -> ListView
    where
        T: Clone + PartialEq + 'static,
        K: Eq + Hash + Clone + 'static,
    {
        let flickable = Flickable::new();
        let orientation = config.orientation;
        let nodes = self.flickable(parent, viewport, &flickable);
        let count = Signal::new(0);
        let instantiated = Signal::new(0);
        let view_size = Signal::new(Vec2::ZERO);
        let metrics = Rc::new(RefCell::new(Metrics::default()));
        let current_index = Signal::new(0);
        let follow = Rc::new(Cell::new(false));
        let highlight = Signal::new(None);
        let mut host = ListHost {
            viewport: nodes.viewport,
            content: nodes.content,
            flickable: flickable.clone(),
            config,
            model: model.clone(),
            key: Box::new(key),
            delegate: Box::new(delegate),
            entries: HashMap::new(),
            extents: RefCell::new(HashMap::new()),
            metrics: metrics.clone(),
            count: count.clone(),
            instantiated: instantiated.clone(),
            view_size: view_size.clone(),
            order: Vec::new(),
            primed: false,
            leaving: Vec::new(),
            reap: Signal::new(0),
            current_index: current_index.clone(),
            last_current: None,
            follow: follow.clone(),
            highlight_node: highlight.clone(),
            highlight: None,
        };
        if untrack(|| host.sync(self)) {
            self.push_host(Box::new(host));
        }
        ListView {
            nodes,
            flickable,
            count,
            instantiated,
            view_size,
            current_index,
            orientation,
            metrics,
            highlight,
            follow,
        }
    }
}
