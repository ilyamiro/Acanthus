use std::collections::{HashMap, HashSet};
use std::hash::Hash;

use acanthus_core::{ListSignal, Signal, batch, untrack};

use crate::node::{IntoSignal, Node, NodeId};
use crate::scene::Scene;

pub(crate) trait Host {
    fn sync(&mut self, scene: &mut Scene) -> bool;

    fn wants_measure(&self) -> bool {
        false
    }

    fn measure(&self, _scene: &Scene) -> bool {
        false
    }
}

/// One instance created by a repeater: the model value and its position in the model.
pub struct Item<T> {
    /// The model value this instance shows.
    pub value: Signal<T>,
    /// The position of this instance in the model.
    pub index: Signal<usize>,
}

impl<T> Clone for Item<T> {
    fn clone(&self) -> Self {
        Item { value: self.value.clone(), index: self.index.clone() }
    }
}

/// Handle to a repeater created by [`Scene::repeater`]; keeps the instances in sync with a model signal.
#[derive(Clone)]
pub struct Repeater {
    /// The node that parents the generated instances.
    pub anchor: NodeId,
    /// The number of instances currently alive.
    pub count: Signal<usize>,
}

/// Handle to a loader created by [`Scene::loader`]; shows one lazily built item at a time.
#[derive(Clone)]
pub struct Loader {
    /// The node of the currently loaded item, or `None` while nothing is loaded.
    pub item: Signal<Option<NodeId>>,
}

struct Entry<T> {
    node: NodeId,
    item: Item<T>,
}

type Builder = Box<dyn FnMut(&mut Scene, NodeId) -> NodeId>;

type Delegate<T> = Box<dyn FnMut(&mut Scene, NodeId, Item<T>) -> NodeId>;

struct RepeaterHost<T, K> {
    parent: NodeId,
    anchor: NodeId,
    model: ListSignal<T>,
    key: Box<dyn Fn(&T) -> K>,
    delegate: Delegate<T>,
    entries: HashMap<K, Entry<T>>,
    order: Vec<NodeId>,
    count: Signal<usize>,
}

impl<T: Clone + PartialEq + 'static, K: Eq + Hash + Clone + 'static> Host for RepeaterHost<T, K> {
    fn sync(&mut self, scene: &mut Scene) -> bool {
        if !scene.contains(self.anchor) {
            for (_, entry) in self.entries.drain() {
                scene.remove(entry.node);
            }
            return false;
        }
        let values = self.model.get();
        let mut seen = HashSet::new();
        let keyed: Vec<(K, T)> =
            values.into_iter().map(|v| ((self.key)(&v), v)).filter(|(k, _)| seen.insert(k.clone())).collect();
        let stale: Vec<K> = self.entries.keys().filter(|k| !seen.contains(*k)).cloned().collect();
        for key in stale {
            if let Some(entry) = self.entries.remove(&key) {
                scene.remove(entry.node);
            }
        }
        let mut order = Vec::with_capacity(keyed.len());
        batch(|| {
            for (index, (key, value)) in keyed.into_iter().enumerate() {
                if let Some(entry) = self.entries.get(&key) {
                    entry.item.value.set(value);
                    entry.item.index.set(index);
                    order.push(entry.node);
                    continue;
                }
                let item = Item { value: Signal::new(value), index: Signal::new(index) };
                let node = untrack(|| (self.delegate)(scene, self.parent, item.clone()));
                order.push(node);
                self.entries.insert(key, Entry { node, item });
            }
        });
        if order != self.order {
            scene.place_after(self.parent, self.anchor, &order);
            self.order = order;
        }
        self.count.set(self.order.len());
        true
    }
}

struct LoaderHost {
    container: NodeId,
    active: Signal<bool>,
    build: Builder,
    item: Signal<Option<NodeId>>,
}

impl Host for LoaderHost {
    fn sync(&mut self, scene: &mut Scene) -> bool {
        if !scene.contains(self.container) {
            return false;
        }
        let active = self.active.get();
        let current = self.item.get_untracked();
        match (active, current) {
            (true, None) => {
                let node = untrack(|| (self.build)(scene, self.container));
                self.item.set(Some(node));
            }
            (false, Some(node)) => {
                scene.remove(node);
                self.item.set(None);
            }
            (true, Some(node)) if !scene.contains(node) => {
                self.item.set(None);
                let node = untrack(|| (self.build)(scene, self.container));
                self.item.set(Some(node));
            }
            _ => {}
        }
        true
    }
}

impl Scene {
    /// Creates a repeater under `parent` that builds one delegate node per element of `model`, keyed by `key` so that reordering keeps instances alive.
    pub fn repeater<T, K>(
        &mut self,
        parent: NodeId,
        model: &ListSignal<T>,
        key: impl Fn(&T) -> K + 'static,
        delegate: impl FnMut(&mut Scene, NodeId, Item<T>) -> NodeId + 'static,
    ) -> Repeater
    where
        T: Clone + PartialEq + 'static,
        K: Eq + Hash + Clone + 'static,
    {
        let anchor = self.add(parent, Node::item().visible(false));
        let count = Signal::new(0);
        let mut host = RepeaterHost {
            parent,
            anchor,
            model: model.clone(),
            key: Box::new(key),
            delegate: Box::new(delegate),
            entries: HashMap::new(),
            order: Vec::new(),
            count: count.clone(),
        };
        if untrack(|| host.sync(self)) {
            self.push_host(Box::new(host));
        }
        Repeater { anchor, count }
    }

    /// Creates a loader inside `container` that runs `build` while `active` is true and removes the built node when it turns false.
    pub fn loader(
        &mut self,
        container: NodeId,
        active: impl IntoSignal<bool>,
        build: impl FnMut(&mut Scene, NodeId) -> NodeId + 'static,
    ) -> Loader {
        let item = Signal::new(None);
        let mut host =
            LoaderHost { container, active: active.into_signal(), build: Box::new(build), item: item.clone() };
        if untrack(|| host.sync(self)) {
            self.push_host(Box::new(host));
        }
        Loader { item }
    }
}
