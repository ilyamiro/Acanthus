use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};
use std::hash::Hash;
use std::rc::Rc;

use acanthus_anim::{Animation, Easing, Tween};
use acanthus_core::{ListSignal, Signal, Vec2, batch, untrack};

use crate::dynamic::{Host, Item};
use crate::flick::Flickable;
use crate::node::{Node, NodeId, Orientation};
use crate::scene::{FlickableNodes, Scene};

pub const DEFAULT_CACHE_BUFFER: f32 = 320.0;
const MEASURE_EPSILON: f32 = 0.01;
const UNMEASURED_EXTENT: f32 = 1.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ViewPosition {
    Beginning,
    Center,
    End,
    Visible,
    Contain,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ListTransitions {
    pub add_scale_from: f32,
    pub add_scale_ms: u32,
    pub add_opacity_ms: u32,
    pub remove_opacity_ms: u32,
    pub easing: Easing,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ListViewConfig {
    pub orientation: Orientation,
    pub spacing: f32,
    pub cache_buffer: f32,
    pub transitions: Option<ListTransitions>,
}

impl Default for ListViewConfig {
    fn default() -> Self {
        ListViewConfig {
            orientation: Orientation::Vertical,
            spacing: 0.0,
            cache_buffer: DEFAULT_CACHE_BUFFER,
            transitions: None,
        }
    }
}

impl ListViewConfig {
    pub fn horizontal() -> Self {
        ListViewConfig { orientation: Orientation::Horizontal, ..Default::default() }
    }

    pub fn spacing(mut self, spacing: f32) -> Self {
        self.spacing = spacing;
        self
    }

    pub fn cache_buffer(mut self, cache_buffer: f32) -> Self {
        self.cache_buffer = cache_buffer;
        self
    }

    pub fn transitions(mut self, transitions: ListTransitions) -> Self {
        self.transitions = Some(transitions);
        self
    }
}

#[derive(Default)]
struct Metrics {
    starts: Vec<f32>,
    extents: Vec<f32>,
}

#[derive(Clone)]
pub struct ListView {
    pub nodes: FlickableNodes,
    pub flickable: Flickable,
    pub count: Signal<usize>,
    pub instantiated: Signal<usize>,
    pub view_size: Signal<Vec2>,
    orientation: Orientation,
    metrics: Rc<RefCell<Metrics>>,
}

impl ListView {
    pub fn item_range(&self, index: usize) -> Option<(f32, f32)> {
        let metrics = self.metrics.borrow();
        Some((*metrics.starts.get(index)?, *metrics.extents.get(index)?))
    }

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

struct Entry<T> {
    node: NodeId,
    item: Item<T>,
}

type Delegate<T> = Box<dyn FnMut(&mut Scene, NodeId, Item<T>) -> NodeId>;

struct Leaving {
    node: NodeId,
    done: Rc<Cell<bool>>,
}

struct Fade {
    tween: Tween<f32>,
    done: Rc<Cell<bool>>,
    reap: Signal<u32>,
}

impl Animation for Fade {
    fn start(&mut self) {
        self.tween.start();
    }

    fn advance(&mut self, dt: f32) -> Option<f32> {
        let rest = self.tween.advance(dt);
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
    known: HashSet<K>,
    primed: bool,
    leaving: Vec<Leaving>,
    reap: Signal<u32>,
}

impl<T, K> ListHost<T, K> {
    fn vertical(&self) -> bool {
        self.config.orientation == Orientation::Vertical
    }

    fn enter(&self, scene: &Scene, node: NodeId) {
        let Some(transitions) = self.config.transitions else { return };
        let Some(target) = scene.get(node) else { return };
        if transitions.add_scale_ms > 0 {
            let scale = target.scale.clone();
            let tween = Tween::new(&scale, 1.0, transitions.add_scale_ms, transitions.easing);
            acanthus_anim::start(tween.from(transitions.add_scale_from));
        }
        if transitions.add_opacity_ms > 0 {
            let opacity = target.opacity.clone();
            let tween = Tween::new(&opacity, 1.0, transitions.add_opacity_ms, transitions.easing);
            acanthus_anim::start(tween.from(0.0));
        }
    }

    fn retire(&mut self, scene: &mut Scene, node: NodeId) {
        let ms = self.config.transitions.map_or(0, |t| t.remove_opacity_ms);
        let (Some(target), true) = (scene.get(node), ms > 0) else {
            scene.remove(node);
            return;
        };
        let opacity = target.opacity.clone();
        let easing = self.config.transitions.map_or(Easing::default(), |t| t.easing);
        let done = Rc::new(Cell::new(false));
        let tween = Tween::new(&opacity, 0.0, ms, easing);
        acanthus_anim::start(Fade { tween, done: done.clone(), reap: self.reap.clone() });
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
                self.retire(scene, entry.node);
            }
        }
        self.extents.borrow_mut().retain(|k, _| seen.contains(k));
        let added: HashSet<K> =
            if self.primed { seen.difference(&self.known).cloned().collect() } else { HashSet::new() };
        self.known = seen.clone();
        self.primed = true;

        let measured: Vec<f32> = {
            let extents = self.extents.borrow();
            keyed.iter().filter_map(|(k, _)| extents.get(k).copied()).collect()
        };
        let estimate = (!measured.is_empty())
            .then(|| (measured.iter().sum::<f32>() / measured.len() as f32).max(UNMEASURED_EXTENT));
        let spacing = self.config.spacing;
        let mut starts = Vec::with_capacity(keyed.len());
        let mut extents = Vec::with_capacity(keyed.len());
        let mut cursor = 0.0;
        {
            let known = self.extents.borrow();
            for (key, _) in &keyed {
                let extent = known.get(key).copied().or(estimate).unwrap_or(UNMEASURED_EXTENT);
                starts.push(cursor);
                extents.push(extent);
                cursor += extent + spacing;
            }
        }
        let total = if keyed.is_empty() { 0.0 } else { cursor - spacing };

        let vertical = self.vertical();
        let view = self.view_size.get();
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

        let wanted: HashSet<K> = keyed[first..last].iter().map(|(k, _)| k.clone()).collect();
        let out_of_range: Vec<K> = self.entries.keys().filter(|k| !wanted.contains(*k)).cloned().collect();
        for key in out_of_range {
            if let Some(entry) = self.entries.remove(&key) {
                scene.remove(entry.node);
            }
        }

        batch(|| {
            for (index, (key, value)) in keyed.into_iter().enumerate() {
                if index < first || index >= last {
                    continue;
                }
                let entry = match self.entries.get(&key) {
                    Some(entry) => {
                        entry.item.value.set(value);
                        entry.item.index.set(index);
                        entry.node
                    }
                    None => {
                        let item = Item { value: Signal::new(value), index: Signal::new(index) };
                        let node = untrack(|| (self.delegate)(scene, self.content, item.clone()));
                        if added.contains(&key) {
                            self.enter(scene, node);
                        }
                        self.entries.insert(key, Entry { node, item });
                        node
                    }
                };
                if let Some(node) = scene.get(entry) {
                    if vertical {
                        node.y.set(starts[index]);
                    } else {
                        node.x.set(starts[index]);
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
        {
            let mut metrics = self.metrics.borrow_mut();
            metrics.starts = starts;
            metrics.extents = extents;
        }
        true
    }
}

impl Scene {
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
        let nodes = self.flickable(parent, viewport, &flickable);
        let count = Signal::new(0);
        let instantiated = Signal::new(0);
        let view_size = Signal::new(Vec2::ZERO);
        let metrics = Rc::new(RefCell::new(Metrics::default()));
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
            known: HashSet::new(),
            primed: false,
            leaving: Vec::new(),
            reap: Signal::new(0),
        };
        if untrack(|| host.sync(self)) {
            self.push_host(Box::new(host));
        }
        ListView { nodes, flickable, count, instantiated, view_size, orientation: config.orientation, metrics }
    }
}
