use acanthus_core::{Affine2, Rect, Signal, Tracker, Vec2};
use slotmap::{SecondaryMap, SlotMap};
use std::rc::Rc;
use std::time::Instant;

mod drag;

use crate::display::{DisplayCx, Primitive};
use crate::dynamic::Host;
use crate::flick::Flickable;
use crate::input::{MouseButton, ScrollDelta};
use crate::keyboard::{Clipboard, CursorShape, KeyEvent, PointerEvent, PointerPhase};
use crate::layout::{ImageEntry, LayoutCx};
use crate::node::{Node, NodeId, paint_order};
use crate::text::{Fonts, ShapedText};
use acanthus_image::ImageStore;

const SETTLE_PASSES: usize = 4;

/// The result of [`Scene::frame`].
pub struct Frame<'a> {
    /// True when the display list differs from the previous frame.
    pub changed: bool,
    /// The display list in paint order.
    pub primitives: &'a [Primitive],
}

/// Layer-surface placement derived from the scene's layer settings: anchored edges, size, margin as left, top, right, bottom and exclusive zone.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LayerGeometry {
    /// Anchored to the top edge.
    pub top: bool,
    /// Anchored to the bottom edge.
    pub bottom: bool,
    /// Anchored to the left edge.
    pub left: bool,
    /// Anchored to the right edge.
    pub right: bool,
    /// Requested size in logical pixels, zero meaning stretch.
    pub size: (u32, u32),
    /// Margins as left, top, right, bottom.
    pub margin: [i32; 4],
    /// Exclusive zone in logical pixels.
    pub exclusive_zone: i32,
}

/// The output a surface is shown on.
#[derive(Debug, Clone, PartialEq)]
pub struct SurfaceOutput {
    /// The output name.
    pub name: String,
    /// The output size in logical pixels.
    pub size: Vec2,
}

/// A retained scene: a tree of nodes with reactive properties, layout, hit testing, focus, scrolling and display-list generation. The renderer consumes [`Scene::frame`]; the platform layer feeds pointer, scroll and key events.
pub struct Scene {
    pub(crate) nodes: SlotMap<NodeId, Node>,
    root: NodeId,
    fonts: Fonts,
    images: ImageStore,
    image_entries: SecondaryMap<NodeId, ImageEntry>,
    texts: SecondaryMap<NodeId, ShapedText>,
    geometry: SecondaryMap<NodeId, Rect>,
    pub(crate) viewport: Vec2,
    pub(crate) viewport_signal: Signal<Vec2>,
    pub(crate) input_region: Signal<Option<Vec<Rect>>>,
    keyboard_on_demand: Signal<Option<bool>>,
    layer_raised: Signal<Option<bool>>,
    pub(crate) layer_geometry: Signal<Option<LayerGeometry>>,
    surface_output: Signal<Option<SurfaceOutput>>,
    pub(crate) scale: f32,
    pub(crate) surface_size: Vec2,
    pub(crate) output_scale: f32,
    pub(crate) ui_scale: Signal<f32>,
    pub(crate) units: f32,
    pub(crate) tracker: Rc<Tracker>,
    primitives: Vec<Primitive>,
    pub(crate) pointer: Option<Vec2>,
    hovered: Vec<NodeId>,
    pressed: Option<(NodeId, MouseButton)>,
    flick_drag: Option<NodeId>,
    pointer_capture: Option<NodeId>,
    drag: Option<drag::DragSession>,
    drag_pending: Option<(NodeId, Vec2)>,
    drag_position: Vec2,
    focus: Option<NodeId>,
    window_focus: bool,
    clipboard: Clipboard,
    epoch: Instant,
    hosts: Vec<Box<dyn Host>>,
}

impl Scene {
    /// Creates a scene with its own image store and the given fonts.
    pub fn new(fonts: Fonts) -> Self {
        Scene::with_images(fonts, ImageStore::new())
    }

    /// Creates a scene sharing an existing image store.
    pub fn with_images(fonts: Fonts, images: ImageStore) -> Self {
        let mut nodes = SlotMap::with_key();
        let root = nodes.insert(Node::item());
        Scene {
            nodes,
            root,
            fonts,
            images,
            image_entries: SecondaryMap::new(),
            texts: SecondaryMap::new(),
            geometry: SecondaryMap::new(),
            viewport: Vec2::ZERO,
            viewport_signal: Signal::new(Vec2::ZERO),
            input_region: Signal::new(None),
            keyboard_on_demand: Signal::new(None),
            layer_raised: Signal::new(None),
            layer_geometry: Signal::new(None),
            surface_output: Signal::new(None),
            scale: 1.0,
            surface_size: Vec2::ZERO,
            output_scale: 1.0,
            ui_scale: Signal::new(1.0),
            units: 1.0,
            tracker: Rc::new(Tracker::new()),
            primitives: Vec::new(),
            pointer: None,
            hovered: Vec::new(),
            pressed: None,
            flick_drag: None,
            pointer_capture: None,
            drag: None,
            drag_pending: None,
            drag_position: Vec2::ZERO,
            focus: None,
            window_focus: true,
            clipboard: Clipboard::new(),
            epoch: Instant::now(),
            hosts: Vec::new(),
        }
    }

    /// The root node, which always exists and fills the viewport.
    pub fn root(&self) -> NodeId {
        self.root
    }

    /// The font database.
    pub fn fonts(&self) -> &Fonts {
        &self.fonts
    }

    /// The image store.
    pub fn images(&self) -> &ImageStore {
        &self.images
    }

    /// Adds `node` as the last child of `parent` and returns its id.
    pub fn add(&mut self, parent: NodeId, mut node: Node) -> NodeId {
        node.parent = Some(parent);
        let id = self.nodes.insert(node);
        self.nodes[parent].children.push(id);
        self.tracker.mark_dirty();
        id
    }

    /// Removes a node and its subtree; unknown ids are ignored.
    pub fn remove(&mut self, id: NodeId) {
        if id == self.root {
            return;
        }
        if let Some(parent) = self.nodes.get(id).and_then(|n| n.parent) {
            self.nodes[parent].children.retain(|c| *c != id);
        }
        let mut stack = vec![id];
        let mut removed = Vec::new();
        while let Some(n) = stack.pop() {
            if let Some(node) = self.nodes.remove(n) {
                stack.extend_from_slice(&node.children);
                self.texts.remove(n);
                self.image_entries.remove(n);
                self.geometry.remove(n);
                removed.push(node);
            }
        }
        self.hovered.retain(|h| self.nodes.contains_key(*h));
        if self.pressed.is_some_and(|(p, _)| !self.nodes.contains_key(p)) {
            self.pressed = None;
        }
        if self.pointer_capture.is_some_and(|p| !self.nodes.contains_key(p)) {
            self.pointer_capture = None;
        }
        if self.focus.is_some_and(|f| !self.nodes.contains_key(f)) {
            self.focus = None;
        }
        self.tracker.mark_dirty();
        drop(removed);
    }

    pub(crate) fn contains(&self, id: NodeId) -> bool {
        self.nodes.contains_key(id)
    }

    pub(crate) fn push_host(&mut self, host: Box<dyn Host>) {
        self.hosts.push(host);
        self.tracker.mark_dirty();
    }

    pub(crate) fn place_after(&mut self, parent: NodeId, anchor: NodeId, ids: &[NodeId]) {
        let moving: std::collections::HashSet<NodeId> = ids.iter().copied().collect();
        let children = &mut self.nodes[parent].children;
        children.retain(|c| !moving.contains(c));
        let at = children.iter().position(|c| *c == anchor).map_or(children.len(), |p| p + 1);
        children.splice(at..at, ids.iter().copied());
        self.tracker.mark_dirty();
    }

    fn sync_hosts(&mut self) {
        if self.hosts.is_empty() {
            return;
        }
        let tracker = self.tracker.clone();
        tracker.track(|| {
            let mut hosts = std::mem::take(&mut self.hosts);
            hosts.retain_mut(|host| host.sync(self));
            hosts.append(&mut self.hosts);
            self.hosts = hosts;
        });
    }

    /// The node with this id, if it still exists.
    pub fn get(&self, id: NodeId) -> Option<&Node> {
        self.nodes.get(id)
    }

    /// Mutable access to the node with this id.
    pub fn get_mut(&mut self, id: NodeId) -> Option<&mut Node> {
        self.tracker.mark_dirty();
        self.nodes.get_mut(id)
    }

    /// The rectangle layout gave the node, or `None` before the first layout or for unknown ids.
    pub fn geometry(&self, id: NodeId) -> Option<Rect> {
        self.geometry.get(id).copied()
    }

    /// The number of nodes including the root.
    pub fn len(&self) -> usize {
        self.nodes.len()
    }

    /// True when only the root exists.
    pub fn is_empty(&self) -> bool {
        self.nodes.len() <= 1
    }

    /// Sets the surface size in compositor logical pixels and the output scale factor, marking the scene dirty on change. The viewport the tree sees is this size divided by the [UI scale](Scene::set_ui_scale).
    pub fn set_viewport(&mut self, size: Vec2, scale: f32) {
        if self.surface_size != size || self.output_scale != scale {
            self.surface_size = size;
            self.output_scale = scale;
            self.apply_units();
        }
    }

    /// The viewport size signal.
    pub fn viewport(&self) -> Signal<Vec2> {
        self.viewport_signal.clone()
    }

    /// The input region the surface should advertise, or `None` for the whole surface.
    pub fn input_region(&self) -> Signal<Option<Vec<Rect>>> {
        self.input_region.clone()
    }

    /// The layer-surface placement requested by the scene, if any.
    pub fn layer_geometry(&self) -> Signal<Option<LayerGeometry>> {
        self.layer_geometry.clone()
    }

    /// The output the scene is shown on, if known.
    pub fn surface_output(&self) -> Signal<Option<SurfaceOutput>> {
        self.surface_output.clone()
    }

    /// Whether the surface wants keyboard focus on demand, `None` when not requested.
    pub fn keyboard_on_demand(&self) -> Signal<Option<bool>> {
        self.keyboard_on_demand.clone()
    }

    /// Whether the layer surface should be raised above others, `None` when not requested.
    pub fn layer_raised(&self) -> Signal<Option<bool>> {
        self.layer_raised.clone()
    }

    /// Device pixels per scene unit: the output scale times the UI scale.
    pub fn scale(&self) -> f32 {
        self.scale
    }

    /// True when something changed since the last [`Scene::frame`].
    pub fn needs_frame(&self) -> bool {
        self.tracker.is_dirty()
    }

    /// Updates layout and returns the display list; cheap and unchanged when nothing is dirty.
    pub fn frame(&mut self) -> Frame<'_> {
        if !self.needs_frame() {
            return Frame { changed: false, primitives: &self.primitives };
        }
        self.apply_units();
        self.sync_hosts();
        self.settle_hosts();
        let viewport = Rect::new(0.0, 0.0, self.viewport.x, self.viewport.y);
        let tracker = self.tracker.clone();
        let next = tracker.track(|| {
            self.ui_scale.get();
            self.run_layout(viewport);
            let mut next = Vec::with_capacity(self.primitives.len());
            DisplayCx {
                nodes: &self.nodes,
                geometry: &self.geometry,
                texts: &self.texts,
                images: &self.image_entries,
                scale: self.scale,
                out: &mut next,
                extent: None,
                forced: None,
                smooth: false,
                snap: self.units != 1.0,
            }
            .build(self.root, viewport);
            next
        });
        let changed = next != self.primitives;
        self.primitives = next;
        Frame { changed, primitives: &self.primitives }
    }

    fn run_layout(&mut self, viewport: Rect) {
        let mut fonts = self.fonts.system();
        LayoutCx {
            images: &mut self.image_entries,
            store: &self.images,
            scale: self.scale,
            nodes: &self.nodes,
            texts: &mut self.texts,
            fonts: &mut fonts,
            geometry: &mut self.geometry,
            implicit: SecondaryMap::new(),
            font_generation: self.fonts.generation().get(),
        }
        .run(self.root, viewport);
    }

    fn settle_hosts(&mut self) {
        if !self.hosts.iter().any(|host| host.wants_measure()) {
            return;
        }
        let tracker = self.tracker.clone();
        let viewport = Rect::new(0.0, 0.0, self.viewport.x, self.viewport.y);
        for _ in 0..SETTLE_PASSES {
            tracker.track(|| self.run_layout(viewport));
            let hosts = std::mem::take(&mut self.hosts);
            let mut changed = false;
            for host in &hosts {
                changed |= tracker.track(|| host.measure(self));
            }
            let mut hosts = hosts;
            hosts.append(&mut self.hosts);
            self.hosts = hosts;
            if !changed {
                break;
            }
            self.sync_hosts();
        }
    }

    fn hits(&self, pos: Vec2) -> Vec<NodeId> {
        let mut out = Vec::new();
        self.collect_hits(self.root, pos, Affine2::IDENTITY, &mut out);
        out
    }

    fn collect_hits(&self, id: NodeId, pos: Vec2, parent: Affine2, out: &mut Vec<NodeId>) {
        let node = &self.nodes[id];
        if !node.visible.get_untracked() {
            return;
        }
        let Some(rect) = self.geometry.get(id).copied() else { return };
        let world = match node.local_transform(rect, false) {
            Some(local) => parent * local,
            None => parent,
        };
        if world.matrix2.determinant() == 0.0 {
            return;
        }
        let inside = rect.contains(world.inverse().transform_point2(pos));
        let interactive = node.mouse.is_some()
            || node.flick.is_some()
            || node.cursor.is_some()
            || node.focusable
            || node.pointer.is_some()
            || node.drop.is_some()
            || node.drag_source.is_some();
        if interactive && inside {
            out.push(id);
        }
        if node.clip.get_untracked() && !inside {
            return;
        }
        for &child in paint_order(&self.nodes, &node.children, false).iter() {
            self.collect_hits(child, pos, world, out);
        }
    }

    fn world_transform(&self, id: NodeId) -> Option<Affine2> {
        let mut chain = Vec::new();
        let mut current = Some(id);
        while let Some(n) = current {
            let node = self.nodes.get(n)?;
            chain.push(n);
            current = node.parent;
        }
        let mut world = Affine2::IDENTITY;
        for n in chain.iter().rev() {
            let rect = self.geometry.get(*n).copied()?;
            if let Some(local) = self.nodes[*n].local_transform(rect, false) {
                world *= local;
            }
        }
        (world.matrix2.determinant() != 0.0).then_some(world)
    }

    fn contains_point(&self, id: NodeId, pos: Vec2) -> bool {
        let Some(world) = self.world_transform(id) else { return false };
        self.geometry.get(id).is_some_and(|r| r.contains(world.inverse().transform_point2(pos)))
    }

    fn local_point(&self, id: NodeId, pos: Vec2) -> Option<Vec2> {
        let world = self.world_transform(id)?;
        let rect = self.geometry.get(id).copied()?;
        let local = world.inverse().transform_point2(pos);
        Some(Vec2::new(local.x - rect.x, local.y - rect.y))
    }

    fn now(&self) -> f32 {
        self.epoch.elapsed().as_secs_f32()
    }

    fn viewport_size(&self, id: NodeId) -> Vec2 {
        self.geometry.get(id).map_or(Vec2::ZERO, |r| Vec2::new(r.w, r.h))
    }

    fn release_flick(&mut self, time: f32) {
        let Some(id) = self.flick_drag.take() else { return };
        let viewport = self.viewport_size(id);
        if let Some(flick) = self.nodes.get(id).and_then(|n| n.flick.clone()) {
            flick.release(time, viewport);
        }
    }

    /// Moves the pointer to `pos` in compositor logical pixels, timestamped from the scene's own clock.
    pub fn pointer_motion(&mut self, pos: Vec2) {
        let time = self.now();
        self.pointer_motion_at(pos, time);
    }

    /// Moves the pointer to `pos` (compositor logical pixels) at `time` seconds, updating hover and running drags.
    pub fn pointer_motion_at(&mut self, pos: Vec2, time: f32) {
        let pos = pos / self.units;
        self.pointer = Some(pos);
        if self.drag_pointer_moved(pos) {
            let now = self.hits(pos);
            for id in &self.hovered {
                if !now.contains(id)
                    && let Some(m) = self.nodes.get(*id).and_then(|n| n.mouse.as_ref())
                {
                    m.hovered.set(false);
                }
            }
            self.hovered = now;
            return;
        }
        if let Some(id) = self.flick_drag {
            let viewport = self.viewport_size(id);
            let flick = self.nodes.get(id).and_then(|n| n.flick.clone());
            if flick.is_some_and(|f| f.drag_to(pos, time, viewport))
                && let Some((target, _)) = self.pressed.take()
                && let Some(m) = self.nodes.get(target).and_then(|n| n.mouse.as_ref())
            {
                m.pressed.set(false);
            }
        }
        if let Some(id) = self.pointer_capture
            && let Some(position) = self.local_point(id, pos)
            && let Some(f) = self.nodes.get_mut(id).and_then(|n| n.pointer.as_mut())
        {
            f(PointerEvent { phase: PointerPhase::Drag, position, time });
        }
        let now = self.hits(pos);
        for id in &self.hovered {
            if !now.contains(id)
                && let Some(m) = self.nodes.get(*id).and_then(|n| n.mouse.as_ref())
            {
                m.hovered.set(false);
            }
        }
        for id in &now {
            if let Some(m) = self.nodes[*id].mouse.as_ref() {
                m.hovered.set(true);
            }
        }
        self.hovered = now;
    }

    /// Reports that the pointer left the surface, ending hover and releasing drags.
    pub fn pointer_leave(&mut self) {
        let time = self.now();
        self.release_flick(time);
        if let Some(pos) = self.pointer {
            self.release_capture(pos, time);
        }
        self.pointer = None;
        self.drag_pending = None;
        for id in self.hovered.drain(..) {
            if let Some(m) = self.nodes.get(id).and_then(|n| n.mouse.as_ref()) {
                m.hovered.set(false);
            }
        }
        if let Some((id, _)) = self.pressed.take()
            && let Some(m) = self.nodes.get(id).and_then(|n| n.mouse.as_ref())
        {
            m.pressed.set(false);
        }
    }

    /// Presses or releases a button, timestamped from the scene's own clock.
    pub fn pointer_button(&mut self, button: MouseButton, down: bool) {
        let time = self.now();
        self.pointer_button_at(button, down, time);
    }

    /// Presses or releases a button at `time` seconds; a release over the node that took the press counts as a click.
    pub fn pointer_button_at(&mut self, button: MouseButton, down: bool, time: f32) {
        let Some(pos) = self.pointer else { return };
        if down {
            let hits = self.hits(pos);
            if button == MouseButton::Left {
                self.drag_candidate(&hits, pos);
                let flick = hits.iter().rev().copied().find(|id| {
                    let viewport = self.viewport_size(*id);
                    self.nodes[*id].flick.as_ref().is_some_and(|f| f.can_flick(viewport))
                });
                if let Some(id) = flick {
                    if let Some(f) = self.nodes[id].flick.as_ref() {
                        f.press(pos, time);
                    }
                    self.flick_drag = Some(id);
                }
            }
            let focus_target = hits.iter().rev().copied().find(|id| self.nodes[*id].focusable);
            self.set_focus(focus_target);
            if button == MouseButton::Left
                && let Some(&capture) = hits.iter().rev().find(|id| self.nodes[**id].pointer.is_some())
            {
                self.pointer_capture = Some(capture);
                if let Some(position) = self.local_point(capture, pos)
                    && let Some(f) = self.nodes[capture].pointer.as_mut()
                {
                    f(PointerEvent { phase: PointerPhase::Press, position, time });
                }
            }
            let Some(&target) = hits.iter().rev().find(|id| self.nodes[**id].mouse.is_some()) else { return };
            if let Some(m) = self.nodes[target].mouse.as_ref() {
                m.pressed.set(true);
            }
            self.pressed = Some((target, button));
            return;
        }
        if button == MouseButton::Left {
            self.release_flick(time);
            self.release_capture(pos, time);
            if self.drag_pointer_released(pos) {
                return;
            }
        }
        let Some((target, pressed_button)) = self.pressed.take() else { return };
        let inside = self.contains_point(target, pos);
        if let Some(m) = self.nodes.get_mut(target).and_then(|n| n.mouse.as_mut()) {
            m.pressed.set(false);
            if inside
                && pressed_button == button
                && let Some(f) = m.on_click.as_mut()
            {
                f(button);
            }
        }
    }

    fn release_capture(&mut self, pos: Vec2, time: f32) {
        let Some(id) = self.pointer_capture.take() else { return };
        let position = self.local_point(id, pos).unwrap_or_default();
        if let Some(f) = self.nodes.get_mut(id).and_then(|n| n.pointer.as_mut()) {
            f(PointerEvent { phase: PointerPhase::Release, position, time });
        }
    }

    /// Delivers a scroll step to the topmost scrollable node under the pointer.
    pub fn scroll(&mut self, delta: ScrollDelta) {
        let delta = ScrollDelta { x: delta.x / self.units, y: delta.y / self.units };
        let Some(pos) = self.pointer else { return };
        let hits = self.hits(pos);
        let target = hits.iter().rev().copied().find(|id| {
            let node = &self.nodes[*id];
            node.mouse.as_ref().is_some_and(|m| m.on_scroll.is_some()) || node.flick.is_some()
        });
        let Some(id) = target else { return };
        if let Some(flick) = self.nodes[id].flick.clone() {
            let viewport = self.viewport_size(id);
            flick.wheel(delta, viewport);
            return;
        }
        if let Some(f) = self.nodes[id].mouse.as_mut().and_then(|m| m.on_scroll.as_mut()) {
            f(delta);
        }
    }

    /// The shared clipboard.
    pub fn clipboard(&self) -> Clipboard {
        self.clipboard.clone()
    }

    /// Routes the scene's clipboard through the system clipboard or another backend.
    pub fn set_clipboard_backend(&self, backend: std::rc::Rc<dyn crate::ClipboardBackend>) {
        self.clipboard.set_backend(backend);
    }

    /// The node with keyboard focus.
    pub fn focused(&self) -> Option<NodeId> {
        self.focus
    }

    /// Moves keyboard focus to `id`; nodes that are not focusable clear it.
    pub fn set_focus(&mut self, id: Option<NodeId>) {
        let id = id.filter(|i| self.nodes.get(*i).is_some_and(|n| n.focusable));
        if id == self.focus {
            return;
        }
        if let Some(old) = self.focus
            && let Some(node) = self.nodes.get(old)
        {
            node.active_focus.set(false);
        }
        self.focus = id;
        if let (Some(new), true) = (id, self.window_focus) {
            self.nodes[new].active_focus.set(true);
        }
    }

    /// Sets whether the window has focus, which controls the active-focus signals.
    pub fn set_window_focus(&mut self, focused: bool) {
        if self.window_focus == focused {
            return;
        }
        self.window_focus = focused;
        if let Some(node) = self.focus.and_then(|f| self.nodes.get(f)) {
            node.active_focus.set(focused);
        }
    }

    /// True when the window has focus.
    pub fn window_focus(&self) -> bool {
        self.window_focus
    }

    /// Moves focus to the next (or, with `backwards`, previous) focusable visible node, wrapping around.
    pub fn focus_next(&mut self, backwards: bool) {
        let mut order = Vec::new();
        self.collect_focusable(self.root, &mut order);
        if order.is_empty() {
            return;
        }
        let current = self.focus.and_then(|f| order.iter().position(|n| *n == f));
        let next = match (current, backwards) {
            (None, false) => 0,
            (None, true) => order.len() - 1,
            (Some(i), false) => (i + 1) % order.len(),
            (Some(i), true) => (i + order.len() - 1) % order.len(),
        };
        self.set_focus(Some(order[next]));
    }

    fn collect_focusable(&self, id: NodeId, out: &mut Vec<NodeId>) {
        let node = &self.nodes[id];
        if !node.visible.get_untracked() {
            return;
        }
        if node.focusable {
            out.push(id);
        }
        for &child in &node.children {
            self.collect_focusable(child, out);
        }
    }

    /// Delivers a key event to the focused node and its ancestors; Tab moves focus when nobody handles it. Returns true when the event was consumed.
    pub fn key_event(&mut self, event: &KeyEvent) -> bool {
        if self.drag.as_ref().is_some_and(|d| d.is_internal()) && event.is(crate::keyboard::Key::Escape) {
            self.drag_cancel();
            return true;
        }
        let mut current = Some(self.focus.unwrap_or(self.root));
        while let Some(id) = current {
            let Some(node) = self.nodes.get_mut(id) else { return false };
            if let Some(handler) = node.key.as_mut()
                && handler(event)
            {
                return true;
            }
            current = node.parent;
        }
        if event.is(crate::keyboard::Key::Tab) && !event.command() {
            self.focus_next(event.modifiers.shift);
            return true;
        }
        false
    }

    /// The cursor shape of the topmost hovered node that requests one.
    pub fn cursor_shape(&self) -> CursorShape {
        self.hovered.iter().rev().find_map(|id| self.nodes.get(*id).and_then(|n| n.cursor)).unwrap_or_default()
    }

    /// Adds a clipping `viewport` under `parent` with a scrolling content node driven by `flickable`.
    pub fn flickable(&mut self, parent: NodeId, viewport: Node, flickable: &Flickable) -> FlickableNodes {
        let mut viewport = viewport.clip(true);
        viewport.flick = Some(flickable.clone());
        let viewport = self.add(parent, viewport);
        let content = Node::item().width(flickable.content_width.clone()).height(flickable.content_height.clone());
        let (cx, cy) = (flickable.content_x.clone(), flickable.content_y.clone());
        let bind_x = content.x.bind(move || -cx.get());
        let bind_y = content.y.bind(move || -cy.get());
        let content = self.add(viewport, content.bind(bind_x).bind(bind_y));
        FlickableNodes { viewport, content }
    }
}

/// The nodes created by [`Scene::flickable`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FlickableNodes {
    /// The clipping viewport.
    pub viewport: NodeId,
    /// The content node that moves as the flickable scrolls.
    pub content: NodeId,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        Align, Anchors, Edge, Elide, FillMode, HAlign, ImageDraw, ImageStatus, LayerEffect, Layout, Margins, MouseArea,
        Quad, Repeater, TransformOrigin, VAlign, WrapMode,
    };
    use acanthus_core::{Color, ListSignal, Signal};
    use std::cell::{Cell, RefCell};
    use std::rc::Rc;

    fn scene(w: f32, h: f32) -> Scene {
        let mut s = Scene::new(Fonts::new());
        s.set_viewport(Vec2::new(w, h), 1.0);
        s
    }

    #[test]
    fn anchors_fill_and_center() {
        let mut s = scene(200.0, 100.0);
        let root = s.root();
        let bg = s.add(root, Node::rect().anchors(Anchors::fill(10.0)));
        let dot = s.add(bg, Node::rect().size(20.0, 20.0).anchors(Anchors::center()));
        let right = s.add(bg, Node::rect().size(30.0, 10.0).anchors(Anchors::default().right(5.0).top(0.0)));
        s.frame();
        assert_eq!(s.geometry(bg), Some(Rect::new(10.0, 10.0, 180.0, 80.0)));
        assert_eq!(s.geometry(dot), Some(Rect::new(90.0, 40.0, 20.0, 20.0)));
        assert_eq!(s.geometry(right), Some(Rect::new(155.0, 10.0, 30.0, 10.0)));
    }

    #[test]
    fn row_layout_with_fill_and_alignment() {
        let mut s = scene(300.0, 40.0);
        let root = s.root();
        let row = s.add(root, Node::item().layout(Layout::row(10.0).padding(5.0, 5.0)).anchors(Anchors::fill(0.0)));
        let a = s.add(row, Node::rect().size(50.0, 20.0));
        let b = s.add(row, Node::rect().height(10.0).fill_width());
        let c = s.add(row, Node::rect().size(40.0, 30.0));
        s.frame();
        assert_eq!(s.geometry(a), Some(Rect::new(5.0, 10.0, 50.0, 20.0)));
        assert_eq!(s.geometry(b), Some(Rect::new(65.0, 15.0, 180.0, 10.0)));
        assert_eq!(s.geometry(c), Some(Rect::new(255.0, 5.0, 40.0, 30.0)));
    }

    #[test]
    fn column_implicit_size_includes_padding() {
        let mut s = scene(500.0, 500.0);
        let root = s.root();
        let col = s.add(root, Node::rect().layout(Layout::column(4.0).padding(8.0, 8.0)));
        s.add(col, Node::rect().size(100.0, 20.0));
        s.add(col, Node::rect().size(60.0, 30.0));
        s.frame();
        assert_eq!(s.geometry(col), Some(Rect::new(0.0, 0.0, 116.0, 70.0)));
    }

    #[test]
    fn loading_a_font_invalidates_the_frame() {
        let mut s = scene(500.0, 100.0);
        let root = s.root();
        let t = s.add(root, Node::text("Acanthus").font_size(24.0).font_family("JetBrains Mono"));
        s.frame();
        assert!(!s.needs_frame());
        let font =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/fonts/JetBrainsMono-Regular.ttf");
        let families = s.fonts().load_file(&font);
        assert!(!families.is_empty());
        assert!(s.fonts().has_family(&families[0]));
        assert!(s.needs_frame());
        s.frame();
        assert!(s.geometry(t).unwrap().w > 0.0);
    }

    #[test]
    fn text_measures_nonzero() {
        let mut s = scene(500.0, 100.0);
        let root = s.root();
        let t = s.add(root, Node::text("Acanthus").font_size(16.0));
        let f = s.frame();
        assert!(f.changed);
        assert!(f.primitives.iter().any(|p| matches!(p, Primitive::Glyph(_))));
        let r = s.geometry(t).unwrap();
        assert!(r.w > 30.0 && r.h >= 16.0, "{r:?}");
    }

    #[test]
    fn signals_only_dirty_the_scene_that_reads_them() {
        let mut a = scene(100.0, 100.0);
        let mut b = scene(100.0, 100.0);
        let color = Signal::new(Color::WHITE);
        let a_root = a.root();
        let b_root = b.root();
        a.add(a_root, Node::rect().size(10.0, 10.0).color(&color));
        b.add(b_root, Node::rect().size(10.0, 10.0));
        a.frame();
        b.frame();
        assert!(!a.needs_frame() && !b.needs_frame());
        color.set(Color::BLACK);
        assert!(a.needs_frame());
        assert!(!b.needs_frame());
        assert!(!b.frame().changed);
    }

    fn labelled_row(s: &mut Scene) -> (NodeId, ListSignal<(u32, f32)>, Repeater) {
        let root = s.root();
        let row = s.add(root, Node::item().layout(Layout::Row { spacing: 2.0, padding: Vec2::ZERO }));
        let model = ListSignal::new(vec![(1, 10.0), (2, 20.0), (3, 30.0)]);
        let repeater = s.repeater(
            row,
            &model,
            |item: &(u32, f32)| item.0,
            |scene, parent, item| scene.add(parent, Node::rect().width(item.value.get_untracked().1).height(10.0)),
        );
        (row, model, repeater)
    }

    #[test]
    fn repeater_builds_items_in_order_inside_the_parent_layout() {
        let mut s = scene(200.0, 100.0);
        let (row, _model, repeater) = labelled_row(&mut s);
        s.frame();
        let kids: Vec<NodeId> =
            s.get(row).unwrap().children().iter().copied().filter(|c| *c != repeater.anchor).collect();
        assert_eq!(kids.len(), 3);
        assert_eq!(repeater.count.get_untracked(), 3);
        assert_eq!(s.geometry(kids[0]).unwrap().x, 0.0);
        assert_eq!(s.geometry(kids[1]).unwrap().x, 12.0);
        assert_eq!(s.geometry(kids[2]).unwrap().x, 34.0);
    }

    #[test]
    fn repeater_keeps_node_identity_across_insert_move_and_remove() {
        let mut s = scene(200.0, 100.0);
        let (row, model, repeater) = labelled_row(&mut s);
        s.frame();
        let nodes = |s: &Scene| -> Vec<NodeId> {
            s.get(row).unwrap().children().iter().copied().filter(|c| *c != repeater.anchor).collect()
        };
        let before = nodes(&s);
        model.insert(1, (9, 5.0));
        assert!(s.needs_frame());
        s.frame();
        let after = nodes(&s);
        assert_eq!(after.len(), 4);
        assert_eq!([after[0], after[2], after[3]], [before[0], before[1], before[2]]);
        assert_eq!(s.geometry(after[1]).unwrap().x, 12.0);
        model.move_item(0, 3);
        s.frame();
        let moved = nodes(&s);
        assert_eq!([moved[0], moved[1], moved[2], moved[3]], [after[1], after[2], after[3], after[0]]);
        model.remove(0);
        s.frame();
        assert_eq!(nodes(&s).len(), 3);
        assert!(s.get(after[1]).is_none());
        assert_eq!(repeater.count.get_untracked(), 3);
    }

    #[test]
    fn repeater_updates_item_values_and_indices() {
        let mut s = scene(200.0, 100.0);
        let root = s.root();
        let model = ListSignal::new(vec![(1, 10.0), (2, 20.0)]);
        let seen = Rc::new(RefCell::new(Vec::new()));
        let capture = seen.clone();
        s.repeater(
            root,
            &model,
            |item: &(u32, f32)| item.0,
            move |scene, parent, item| {
                capture.borrow_mut().push(item.clone());
                scene.add(parent, Node::item())
            },
        );
        s.frame();
        model.update_at(1, |entry| entry.1 = 99.0);
        model.move_item(1, 0);
        s.frame();
        let items = seen.borrow();
        assert_eq!(items.len(), 2);
        assert_eq!(items[1].value.get_untracked(), (2, 99.0));
        assert_eq!(items[1].index.get_untracked(), 0);
        assert_eq!(items[0].index.get_untracked(), 1);
    }

    #[test]
    fn repeater_ignores_duplicate_keys_and_stops_with_its_parent() {
        let mut s = scene(200.0, 100.0);
        let root = s.root();
        let parent = s.add(root, Node::item());
        let model = ListSignal::new(vec![1u32, 1, 2]);
        let repeater = s.repeater(parent, &model, |k: &u32| *k, |scene, p, _| scene.add(p, Node::item()));
        s.frame();
        assert_eq!(repeater.count.get_untracked(), 2);
        s.remove(parent);
        let before = s.len();
        model.push(5);
        s.frame();
        assert_eq!(s.len(), before);
    }

    #[test]
    fn nested_repeaters_build_inner_items() {
        let mut s = scene(200.0, 100.0);
        let root = s.root();
        let outer = ListSignal::new(vec![1u32, 2]);
        let inner = ListSignal::new(vec![10u32, 20, 30]);
        let inner_model = inner.clone();
        s.repeater(
            root,
            &outer,
            |k: &u32| *k,
            move |scene, parent, _| {
                let cell = scene.add(parent, Node::item());
                scene.repeater(cell, &inner_model, |k: &u32| *k, |scene, p, _| scene.add(p, Node::item()));
                cell
            },
        );
        s.frame();
        assert_eq!(s.len(), 1 + 1 + 2 * (1 + 1 + 3));
        inner.push(40);
        s.frame();
        assert_eq!(s.len(), 1 + 1 + 2 * (1 + 1 + 4));
    }

    #[test]
    fn loader_builds_and_destroys_on_active() {
        let mut s = scene(200.0, 100.0);
        let root = s.root();
        let active = Signal::new(false);
        let host = s.add(root, Node::item().layout(Layout::Stack { padding: Vec2::ZERO }));
        let loader = s.loader(host, &active, |scene, parent| scene.add(parent, Node::rect().size(30.0, 12.0)));
        s.frame();
        assert_eq!(loader.item.get_untracked(), None);
        assert_eq!(s.get(host).unwrap().children().len(), 0);
        active.set(true);
        assert!(s.needs_frame());
        s.frame();
        let node = loader.item.get_untracked().unwrap();
        assert_eq!(
            {
                let g = s.geometry(host).unwrap();
                Vec2::new(g.w, g.h)
            },
            Vec2::new(30.0, 12.0)
        );
        assert!(s.get(node).is_some());
        active.set(false);
        s.frame();
        assert!(s.get(node).is_none());
        assert_eq!(loader.item.get_untracked(), None);
        active.set(true);
        s.frame();
        assert_ne!(loader.item.get_untracked(), Some(node));
    }

    #[test]
    fn frame_only_rebuilds_when_signals_change() {
        let mut s = scene(100.0, 100.0);
        let root = s.root();
        let color = Signal::new(Color::WHITE);
        s.add(root, Node::rect().size(10.0, 10.0).color(&color));
        assert!(s.frame().changed);
        assert!(!s.needs_frame());
        color.set(Color::BLACK);
        assert!(s.needs_frame());
        assert!(s.frame().changed);
    }

    #[test]
    fn opacity_and_visibility_propagate() {
        let mut s = scene(100.0, 100.0);
        let root = s.root();
        let group = s.add(root, Node::item().opacity(0.5).anchors(Anchors::fill(0.0)));
        s.add(group, Node::rect().size(10.0, 10.0).color(Color::WHITE));
        let hidden = s.add(root, Node::item().visible(false));
        s.add(hidden, Node::rect().size(10.0, 10.0));
        let f = s.frame();
        assert_eq!(f.primitives.len(), 1);
        let Primitive::Quad(q) = f.primitives[0] else { panic!() };
        assert_eq!(q.start.a, 0.5);
    }

    #[test]
    fn hover_and_click() {
        let mut s = scene(100.0, 100.0);
        let root = s.root();
        let clicks = Rc::new(Cell::new(0));
        let c = clicks.clone();
        let area = MouseArea::new().on_click(move |_| c.set(c.get() + 1));
        let hovered = area.hovered();
        s.add(root, Node::rect().size(50.0, 50.0).mouse(area));
        s.frame();
        s.pointer_motion(Vec2::new(10.0, 10.0));
        assert!(hovered.get_untracked());
        s.pointer_button(MouseButton::Left, true);
        s.pointer_button(MouseButton::Left, false);
        assert_eq!(clicks.get(), 1);
        s.pointer_button(MouseButton::Left, true);
        s.pointer_motion(Vec2::new(80.0, 80.0));
        assert!(!hovered.get_untracked());
        s.pointer_button(MouseButton::Left, false);
        assert_eq!(clicks.get(), 1);
    }

    #[test]
    fn clip_restricts_hits() {
        let mut s = scene(100.0, 100.0);
        let root = s.root();
        let clipper = s.add(root, Node::item().size(20.0, 20.0).clip(true));
        let area = MouseArea::new();
        let hovered = area.hovered();
        s.add(clipper, Node::rect().size(80.0, 80.0).mouse(area));
        s.frame();
        s.pointer_motion(Vec2::new(50.0, 50.0));
        assert!(!hovered.get_untracked());
        s.pointer_motion(Vec2::new(10.0, 10.0));
        assert!(hovered.get_untracked());
    }

    #[test]
    fn remove_drops_subtree_and_bindings() {
        let mut s = scene(100.0, 100.0);
        let root = s.root();
        let src = Signal::new(1.0f32);
        let dst = Signal::new(0.0f32);
        let src2 = src.clone();
        let parent = s.add(root, Node::item());
        s.add(parent, Node::rect().bind(dst.bind(move || src2.get() * 2.0)));
        assert_eq!(s.len(), 3);
        s.remove(parent);
        assert_eq!(s.len(), 1);
        src.set(5.0);
        assert_eq!(dst.get_untracked(), 2.0);
    }

    fn glyph_xs(s: &mut Scene) -> Vec<i32> {
        s.frame().primitives.iter().filter_map(|p| if let Primitive::Glyph(g) = p { Some(g.x) } else { None }).collect()
    }

    fn glyph_ys(s: &mut Scene) -> Vec<i32> {
        let mut ys: Vec<i32> = s
            .frame()
            .primitives
            .iter()
            .filter_map(|p| if let Primitive::Glyph(g) = p { Some(g.y) } else { None })
            .collect();
        ys.dedup();
        ys
    }

    #[test]
    fn transformed_text_keeps_its_fractional_vertical_offset() {
        let ty = |y: f32, scale: f32| {
            let mut s = scene(200.0, 100.0);
            let root = s.root();
            s.add(root, Node::text("Hi").font_size(16.0).x(10.0).y(y).size(60.0, 20.0).scale(scale));
            s.frame()
                .primitives
                .iter()
                .find_map(|p| if let Primitive::Glyph(g) = p { Some(g.transform.translation.y) } else { None })
                .unwrap()
        };
        let (a, b) = (ty(10.0, 1.2), ty(10.5, 1.2));
        assert_ne!(a, b);
    }

    #[test]
    fn text_horizontal_alignment_moves_glyphs() {
        let mut xs = Vec::new();
        for align in [HAlign::Left, HAlign::Center, HAlign::Right] {
            let mut s = scene(400.0, 100.0);
            let root = s.root();
            s.add(root, Node::text("Hello").font_size(16.0).size(300.0, 40.0).h_align(align));
            xs.push(*glyph_xs(&mut s).iter().min().unwrap());
        }
        assert!(xs[0] < xs[1] && xs[1] < xs[2], "{xs:?}");
        assert!(xs[0] <= 1);
    }

    #[test]
    fn text_vertical_alignment_moves_glyphs() {
        let mut ys = Vec::new();
        for align in [VAlign::Top, VAlign::Center, VAlign::Bottom] {
            let mut s = scene(400.0, 200.0);
            let root = s.root();
            s.add(root, Node::text("Hello").font_size(16.0).size(100.0, 100.0).v_align(align));
            ys.push(glyph_ys(&mut s)[0]);
        }
        assert!(ys[0] < ys[1] && ys[1] < ys[2], "{ys:?}");
    }

    #[test]
    fn text_wraps_only_with_a_wrap_mode() {
        let long = "one two three four five six seven eight nine ten";
        let mut plain = scene(400.0, 200.0);
        let root = plain.root();
        plain.add(root, Node::text(long).font_size(16.0).size(80.0, 100.0));
        assert_eq!(glyph_ys(&mut plain).len(), 1);

        let mut wrapped = scene(400.0, 200.0);
        let root = wrapped.root();
        wrapped.add(root, Node::text(long).font_size(16.0).size(80.0, 100.0).wrap_mode(WrapMode::Word));
        assert!(glyph_ys(&mut wrapped).len() > 2);

        let mut limited = scene(400.0, 200.0);
        let root = limited.root();
        limited.add(root, Node::text(long).font_size(16.0).size(80.0, 100.0).wrap_mode(WrapMode::Word).max_lines(2));
        assert_eq!(glyph_ys(&mut limited).len(), 2);
    }

    #[test]
    fn text_elides_to_the_available_width() {
        let long = "a very long piece of text that cannot fit";
        let mut full = scene(600.0, 100.0);
        let root = full.root();
        full.add(root, Node::text(long).font_size(16.0));
        let full_right = *glyph_xs(&mut full).iter().max().unwrap();

        let mut cut = scene(600.0, 100.0);
        let root = cut.root();
        cut.add(root, Node::text(long).font_size(16.0).size(100.0, 30.0).elide(Elide::Right));
        let cut_right = *glyph_xs(&mut cut).iter().max().unwrap();
        assert!(cut_right <= 100, "{cut_right}");
        assert!(cut_right < full_right);
    }

    #[test]
    fn text_capitalization_changes_the_shaped_text() {
        let width = |case| {
            let mut s = scene(600.0, 200.0);
            let root = s.root();
            let t = s.add(root, Node::text("illimitable").font_size(16.0).capitalization(case));
            s.frame();
            s.geometry(t).unwrap().w
        };
        assert_eq!(width(crate::Capitalization::MixedCase), width(crate::Capitalization::AllLowercase));
        assert!(width(crate::Capitalization::AllUppercase) > width(crate::Capitalization::MixedCase) + 10.0);
    }

    #[test]
    fn text_letter_spacing_and_line_height_change_metrics() {
        let mut base = scene(600.0, 200.0);
        let root = base.root();
        let t = base.add(root, Node::text("spacing").font_size(16.0));
        base.frame();
        let base_w = base.geometry(t).unwrap().w;

        let mut spaced = scene(600.0, 200.0);
        let root = spaced.root();
        let t = spaced.add(root, Node::text("spacing").font_size(16.0).letter_spacing(4.0));
        spaced.frame();
        assert!(spaced.geometry(t).unwrap().w > base_w + 20.0);

        let mut tall = scene(600.0, 200.0);
        let root = tall.root();
        let t = tall.add(root, Node::text("x").font_size(16.0).line_height(2.0));
        tall.frame();
        assert_eq!(tall.geometry(t).unwrap().h, 40.0);
    }

    #[test]
    fn row_margins_and_preferred_size_apply() {
        let mut s = scene(500.0, 100.0);
        let root = s.root();
        let row = s.add(root, Node::item().layout(Layout::row(10.0)).anchors(Anchors::fill(0.0)));
        let a = s.add(row, Node::rect().preferred(40.0, 20.0).margins(Margins::all(5.0)).align(Align::Start));
        let b = s.add(row, Node::rect().size(30.0, 10.0).align(Align::Start));
        s.frame();
        assert_eq!(s.geometry(a), Some(Rect::new(5.0, 5.0, 40.0, 20.0)));
        assert_eq!(s.geometry(b), Some(Rect::new(60.0, 0.0, 30.0, 10.0)));
    }

    #[test]
    fn fill_width_respects_min_and_max() {
        let mut s = scene(300.0, 50.0);
        let root = s.root();
        let row = s.add(root, Node::item().layout(Layout::row(0.0)).anchors(Anchors::fill(0.0)));
        let capped = s.add(row, Node::rect().height(10.0).fill_width().max_size(50.0, f32::INFINITY));
        let rest = s.add(row, Node::rect().height(10.0).fill_width());
        s.frame();
        assert_eq!(s.geometry(capped).unwrap().w, 50.0);
        assert_eq!(s.geometry(rest).unwrap().w, 250.0);
    }

    #[test]
    fn grid_sizes_columns_and_spans() {
        let mut s = scene(500.0, 500.0);
        let root = s.root();
        let grid = s.add(root, Node::item().layout(Layout::grid(2, 4.0, 6.0)));
        let a = s.add(grid, Node::rect().size(30.0, 10.0).align(Align::Start));
        let b = s.add(grid, Node::rect().size(50.0, 20.0).align(Align::Start));
        let wide = s.add(grid, Node::rect().size(100.0, 10.0).span(2, 1).align(Align::Start));
        let c = s.add(grid, Node::rect().size(20.0, 10.0).align(Align::Start));
        s.frame();
        assert_eq!(s.geometry(a), Some(Rect::new(0.0, 0.0, 30.0, 10.0)));
        assert_eq!(s.geometry(b), Some(Rect::new(42.0, 0.0, 50.0, 20.0)));
        assert_eq!(s.geometry(wide).unwrap().y, 26.0);
        assert_eq!(s.geometry(c).unwrap().x, 0.0);
        assert_eq!(s.geometry(c).unwrap().y, 42.0);
        assert_eq!(s.geometry(grid).unwrap().w, 100.0);
        assert_eq!(s.geometry(grid).unwrap().h, 52.0);
    }

    #[test]
    fn flow_wraps_at_the_available_width() {
        let mut s = scene(500.0, 500.0);
        let root = s.root();
        let flow = s.add(root, Node::item().width(70.0).layout(Layout::flow(5.0, 5.0)));
        let ids: Vec<NodeId> = (0..3).map(|_| s.add(flow, Node::rect().size(30.0, 10.0).align(Align::Start))).collect();
        s.frame();
        assert_eq!(s.geometry(ids[1]).unwrap().x, 35.0);
        assert_eq!(s.geometry(ids[2]), Some(Rect::new(0.0, 15.0, 30.0, 10.0)));
        assert_eq!(s.geometry(flow).unwrap().h, 25.0);
    }

    #[test]
    fn stack_children_fill_and_size_to_the_largest() {
        let mut s = scene(500.0, 500.0);
        let root = s.root();
        let stack = s.add(root, Node::item().layout(Layout::stack()));
        let a = s.add(stack, Node::rect().size(40.0, 30.0));
        let b = s.add(stack, Node::rect().size(10.0, 10.0));
        s.frame();
        assert_eq!(s.geometry(stack), Some(Rect::new(0.0, 0.0, 40.0, 30.0)));
        assert_eq!(s.geometry(a), s.geometry(stack));
        assert_eq!(s.geometry(b), s.geometry(stack));
    }

    #[test]
    fn center_offsets_and_sibling_anchors() {
        let mut s = scene(200.0, 100.0);
        let root = s.root();
        let first = s.add(root, Node::rect().size(20.0, 20.0).anchors(Anchors::default().left(10.0).top(10.0)));
        let second = s.add(
            root,
            Node::rect().size(30.0, 20.0).anchors(Anchors::default().left_to(first, Edge::Right, 4.0).top_to(
                first,
                Edge::Top,
                0.0,
            )),
        );
        let shifted = s.add(root, Node::rect().size(10.0, 10.0).anchors(Anchors::center().h_center_offset(20.0)));
        s.frame();
        assert_eq!(s.geometry(second), Some(Rect::new(34.0, 10.0, 30.0, 20.0)));
        assert_eq!(s.geometry(shifted).unwrap().x, 115.0);
        assert_eq!(s.geometry(shifted).unwrap().y, 45.0);
    }

    #[test]
    fn sibling_anchors_resolve_regardless_of_order() {
        let mut s = scene(200.0, 100.0);
        let root = s.root();
        let placeholder = s.add(root, Node::item());
        let late = s.add(root, Node::rect().size(20.0, 20.0).anchors(Anchors::default().left(50.0).top(0.0)));
        let early = s.add(
            root,
            Node::rect().size(10.0, 10.0).anchors(Anchors::default().left_to(late, Edge::Right, 0.0).top(0.0)),
        );
        let _ = placeholder;
        s.frame();
        assert_eq!(s.geometry(early).unwrap().x, 70.0);
    }

    #[test]
    fn viewport_signal_follows_set_viewport() {
        let mut scene = Scene::new(Fonts::new());
        let viewport = scene.viewport();
        assert_eq!(viewport.get_untracked(), Vec2::ZERO);
        scene.set_viewport(Vec2::new(300.0, 40.0), 1.0);
        assert_eq!(viewport.get_untracked(), Vec2::new(300.0, 40.0));
    }

    #[test]
    fn implicit_size_signal_tracks_layout() {
        let mut s = scene(500.0, 500.0);
        let root = s.root();
        let node = Node::rect().layout(Layout::row(4.0));
        let implicit = node.implicit_size();
        let row = s.add(root, node);
        s.add(row, Node::rect().size(20.0, 10.0));
        s.add(row, Node::rect().size(30.0, 14.0));
        s.frame();
        assert_eq!(implicit.get_untracked(), Vec2::new(54.0, 14.0));
    }

    #[test]
    fn bounds_signal_reports_the_laid_out_rect_in_scene_space() {
        let mut s = scene(500.0, 500.0);
        let root = s.root();
        let row = s.add(root, Node::rect().x(10.0).y(7.0).layout(Layout::row(4.0)));
        s.add(row, Node::rect().size(20.0, 10.0));
        let node = Node::rect().size(30.0, 14.0);
        let bounds = node.bounds();
        s.add(row, node);
        s.frame();
        assert_eq!(bounds.get_untracked(), Rect::new(34.0, 7.0, 30.0, 14.0));
    }

    fn png_source() -> (tempfile::TempDir, String) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("wide.png");
        image::RgbaImage::from_pixel(40, 20, image::Rgba([255, 0, 0, 255])).save(&path).unwrap();
        let source = path.to_str().unwrap().to_owned();
        (dir, source)
    }

    fn wait_for_images(s: &mut Scene) {
        let start = std::time::Instant::now();
        while !s.images().poll() && start.elapsed() < std::time::Duration::from_secs(5) {
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
    }

    #[test]
    fn image_loads_asynchronously_and_draws_with_fill_mode() {
        let (_dir, source) = png_source();
        let mut s = scene(200.0, 200.0);
        let root = s.root();
        let id = s.add(root, Node::image(source.as_str()).fill_mode(FillMode::PreserveAspectFit).size(100.0, 100.0));
        let first = s.frame();
        assert!(first.primitives.iter().all(|p| !matches!(p, Primitive::Image(_))));
        wait_for_images(&mut s);
        assert!(s.needs_frame());
        let f = s.frame();
        let draws: Vec<&ImageDraw> =
            f.primitives.iter().filter_map(|p| if let Primitive::Image(d) = p { Some(d) } else { None }).collect();
        assert_eq!(draws.len(), 1);
        assert_eq!(draws[0].rect, Rect::new(0.0, 25.0, 100.0, 50.0));
        assert_eq!(s.geometry(id), Some(Rect::new(0.0, 0.0, 100.0, 100.0)));
    }

    #[test]
    fn image_implicit_size_and_status_follow_the_decoded_image() {
        let (_dir, source) = png_source();
        let node = Node::image(source.as_str());
        let status = node.image_status().unwrap();
        let implicit = node.implicit_size();
        let mut s = scene(200.0, 200.0);
        let root = s.root();
        s.add(root, node);
        s.frame();
        assert_eq!(status.get_untracked(), ImageStatus::Loading);
        wait_for_images(&mut s);
        s.frame();
        assert_eq!(status.get_untracked(), ImageStatus::Ready);
        assert_eq!(implicit.get_untracked(), Vec2::new(40.0, 20.0));
    }

    #[test]
    fn canvas_repaints_on_resize_and_signal_change_at_the_device_scale() {
        let color = Signal::new(Color::WHITE);
        let width = Signal::new(40.0);
        let runs = Rc::new(Cell::new(0));
        let node = {
            let (color, runs) = (color.clone(), runs.clone());
            Node::canvas(move |p| {
                runs.set(runs.get() + 1);
                let area = Rect::new(0.0, 0.0, p.size.x, p.size.y);
                p.fill(&crate::Path::new().rect(area), color.get());
            })
            .size(&width, 20.0)
        };
        let implicit = node.implicit_size();
        let mut s = Scene::new(Fonts::new());
        s.set_viewport(Vec2::new(100.0, 100.0), 2.0);
        let root = s.root();
        s.add(root, node);
        let drawn = |s: &mut Scene| {
            while s.needs_frame() {
                s.frame();
            }
            let f = s.frame();
            let draw = f.primitives.iter().find_map(|p| if let Primitive::Image(d) = p { Some(*d) } else { None });
            let draw = draw.unwrap();
            let image = s.images().get(draw.id).unwrap();
            (draw, image.width, image.height, image.rgba[..4].to_vec())
        };
        let (first, w, h, px) = drawn(&mut s);
        assert_eq!((w, h, px), (80, 40, vec![255; 4]));
        assert_eq!(first.rect, Rect::new(0.0, 0.0, 80.0, 40.0));
        assert_eq!(implicit.get_untracked(), Vec2::ZERO);
        let painted = runs.get();

        s.frame();
        assert_eq!(runs.get(), painted);
        color.set(Color::BLACK);
        let (second, _, _, px) = drawn(&mut s);
        assert_ne!(second.id, first.id);
        assert_eq!(px, vec![0, 0, 0, 255]);

        width.set(10.0);
        let (_, w, _, _) = drawn(&mut s);
        assert_eq!(w, 20);
    }

    fn gif_source() -> (tempfile::TempDir, String) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("blink.gif");
        let mut out = Vec::new();
        {
            let mut encoder = image::codecs::gif::GifEncoder::new(&mut out);
            for (i, delay) in [100u32, 200].into_iter().enumerate() {
                let color = if i == 0 { [255, 0, 0, 255] } else { [0, 0, 255, 255] };
                let frame = image::Frame::from_parts(
                    image::RgbaImage::from_pixel(8, 8, image::Rgba(color)),
                    0,
                    0,
                    image::Delay::from_numer_denom_ms(delay, 1),
                );
                encoder.encode_frame(frame).unwrap();
            }
        }
        std::fs::write(&path, out).unwrap();
        let source = path.to_str().unwrap().to_owned();
        (dir, source)
    }

    fn image_frames(s: &mut Scene) -> Vec<u32> {
        s.frame()
            .primitives
            .iter()
            .filter_map(|p| if let Primitive::Image(d) = p { Some(d.frame) } else { None })
            .collect()
    }

    #[test]
    fn animated_image_steps_through_its_frames_and_loops() {
        let (_dir, source) = gif_source();
        let mut s = scene(100.0, 100.0);
        let root = s.root();
        s.add(root, Node::image(source.as_str()).animated(true).size(20.0, 20.0));
        s.frame();
        wait_for_images(&mut s);
        assert_eq!(image_frames(&mut s), vec![0]);
        assert!(acanthus_anim::is_active());
        acanthus_anim::tick(0.05);
        assert!(!s.needs_frame());
        acanthus_anim::tick(0.06);
        assert!(s.needs_frame());
        assert_eq!(image_frames(&mut s), vec![1]);
        acanthus_anim::tick(0.2);
        assert_eq!(image_frames(&mut s), vec![0]);
    }

    #[test]
    fn still_image_nodes_ignore_the_animation_and_stop_the_clock_when_dropped() {
        let (_dir, source) = gif_source();
        let mut s = scene(100.0, 100.0);
        let root = s.root();
        let still = s.add(root, Node::image(source.as_str()).size(20.0, 20.0));
        s.frame();
        wait_for_images(&mut s);
        assert_eq!(image_frames(&mut s), vec![0]);
        assert!(!acanthus_anim::is_active());
        s.remove(still);
        let animated = s.add(root, Node::image(source.as_str()).animated(true).size(20.0, 20.0));
        s.frame();
        wait_for_images(&mut s);
        s.frame();
        assert!(acanthus_anim::is_active());
        s.remove(animated);
        s.frame();
        assert!(!acanthus_anim::is_active());
    }

    #[test]
    fn missing_image_reports_an_error_status() {
        let node = Node::image("/no/such/image.png").size(10.0, 10.0);
        let status = node.image_status().unwrap();
        let mut s = scene(50.0, 50.0);
        let root = s.root();
        s.add(root, node);
        s.frame();
        wait_for_images(&mut s);
        s.frame();
        assert_eq!(status.get_untracked(), ImageStatus::Error);
    }

    fn quads(s: &mut Scene) -> Vec<Quad> {
        s.frame().primitives.iter().filter_map(|p| if let Primitive::Quad(q) = p { Some(*q) } else { None }).collect()
    }

    fn assert_near(a: Vec2, b: Vec2) {
        assert!((a - b).length() < 1e-3, "{a:?} != {b:?}");
    }

    #[test]
    fn rotation_turns_clockwise_about_the_center() {
        let mut s = scene(100.0, 100.0);
        let root = s.root();
        s.add(root, Node::rect().size(40.0, 20.0).rotation(90.0));
        let q = quads(&mut s)[0];
        assert_near(q.transform.transform_point2(Vec2::new(40.0, 10.0)), Vec2::new(20.0, 30.0));
        assert_near(q.transform.transform_point2(Vec2::new(20.0, 10.0)), Vec2::new(20.0, 10.0));
    }

    #[test]
    fn transforms_compose_with_the_parent_and_the_origin() {
        let mut s = scene(200.0, 200.0);
        let root = s.root();
        let parent = s.add(root, Node::item().size(100.0, 100.0).scale(2.0).transform_origin(TransformOrigin::TopLeft));
        s.add(
            parent,
            Node::rect().x(10.0).y(10.0).size(20.0, 20.0).scale(0.5).transform_origin(TransformOrigin::TopLeft),
        );
        s.add(root, Node::rect().size(10.0, 10.0));
        let q = quads(&mut s);
        assert_near(q[0].transform.transform_point2(Vec2::new(10.0, 10.0)), Vec2::new(20.0, 20.0));
        assert_near(q[0].transform.transform_point2(Vec2::new(30.0, 30.0)), Vec2::new(40.0, 40.0));
        assert_eq!(q[1].transform, Affine2::IDENTITY);
    }

    #[test]
    fn transforms_scale_to_physical_pixels() {
        let mut s = scene(100.0, 100.0);
        s.set_viewport(Vec2::new(100.0, 100.0), 2.0);
        let root = s.root();
        s.add(root, Node::rect().x(10.0).y(10.0).size(20.0, 20.0).scale(2.0));
        let q = quads(&mut s)[0];
        assert_eq!(q.rect, Rect::new(20.0, 20.0, 40.0, 40.0));
        assert_near(q.transform.transform_point2(Vec2::new(20.0, 20.0)), Vec2::new(0.0, 0.0));
        assert_near(q.transform.transform_point2(Vec2::new(60.0, 60.0)), Vec2::new(80.0, 80.0));
    }

    #[test]
    fn z_orders_siblings_stably() {
        let mut s = scene(100.0, 100.0);
        let root = s.root();
        let z = Signal::new(0.0f32);
        s.add(root, Node::rect().size(10.0, 10.0).color(Color::hex(0x000001)).z(&z));
        s.add(root, Node::rect().size(10.0, 10.0).color(Color::hex(0x000002)).z(-1.0));
        s.add(root, Node::rect().size(10.0, 10.0).color(Color::hex(0x000003)));
        let order = |s: &mut Scene| quads(s).iter().map(|q| (q.start.b * 255.0).round() as u8).collect::<Vec<_>>();
        assert_eq!(order(&mut s), vec![2, 1, 3]);
        z.set(5.0);
        assert!(s.needs_frame());
        assert_eq!(order(&mut s), vec![2, 3, 1]);
    }

    #[test]
    fn higher_z_receives_the_press() {
        let mut s = scene(100.0, 100.0);
        let root = s.root();
        let top = MouseArea::new();
        let bottom = MouseArea::new();
        let top_pressed = top.pressed();
        let bottom_pressed = bottom.pressed();
        s.add(root, Node::rect().size(50.0, 50.0).z(1.0).mouse(top));
        s.add(root, Node::rect().size(50.0, 50.0).mouse(bottom));
        s.frame();
        s.pointer_motion(Vec2::new(10.0, 10.0));
        s.pointer_button(MouseButton::Left, true);
        assert!(top_pressed.get_untracked());
        assert!(!bottom_pressed.get_untracked());
    }

    #[test]
    fn hit_testing_inverts_rotation_and_scale() {
        let mut s = scene(100.0, 100.0);
        let root = s.root();
        let bar = MouseArea::new();
        let bar_hovered = bar.hovered();
        s.add(root, Node::rect().y(40.0).size(100.0, 20.0).rotation(90.0).mouse(bar));
        let dot = MouseArea::new();
        let dot_hovered = dot.hovered();
        s.add(root, Node::rect().x(70.0).y(70.0).size(10.0, 10.0).scale(2.0).mouse(dot));
        s.frame();
        s.pointer_motion(Vec2::new(50.0, 5.0));
        assert!(bar_hovered.get_untracked());
        s.pointer_motion(Vec2::new(10.0, 50.0));
        assert!(!bar_hovered.get_untracked());
        s.pointer_motion(Vec2::new(66.0, 66.0));
        assert!(dot_hovered.get_untracked());
        s.pointer_motion(Vec2::new(64.0, 64.0));
        assert!(!dot_hovered.get_untracked());
    }

    #[test]
    fn clip_follows_the_clipper_transform() {
        let mut s = scene(100.0, 100.0);
        let root = s.root();
        let clipper = s.add(root, Node::item().x(30.0).y(30.0).size(40.0, 40.0).rotation(45.0).clip(true));
        let area = MouseArea::new();
        let hovered = area.hovered();
        s.add(clipper, Node::rect().x(-100.0).y(-100.0).size(300.0, 300.0).mouse(area));
        s.frame();
        s.pointer_motion(Vec2::new(31.0, 31.0));
        assert!(!hovered.get_untracked());
        s.pointer_motion(Vec2::new(50.0, 25.0));
        assert!(hovered.get_untracked());
    }

    #[test]
    fn rounded_and_transformed_clips_carry_a_shape() {
        let mut s = scene(200.0, 100.0);
        let root = s.root();
        let plain = s.add(root, Node::item().size(50.0, 50.0).clip(true));
        s.add(plain, Node::rect().size(80.0, 80.0));
        let rounded = s.add(root, Node::rect().x(60.0).size(50.0, 50.0).radius(12.0).clip(true));
        s.add(rounded, Node::rect().size(80.0, 80.0));
        let turned = s.add(root, Node::item().x(120.0).size(40.0, 40.0).rotation(30.0).clip(true));
        s.add(turned, Node::rect().size(80.0, 80.0));
        let q = quads(&mut s);
        assert_eq!(q[0].clip.shape, None);
        assert_eq!(q[0].clip.rect, Rect::new(0.0, 0.0, 50.0, 50.0));
        let shape = q[2].clip.shape.unwrap();
        assert_eq!(shape.radius, 12.0);
        assert_eq!(shape.rect, Rect::new(60.0, 0.0, 50.0, 50.0));
        assert_eq!(q[2].clip.rect, Rect::new(60.0, 0.0, 50.0, 50.0));
        let shape = q[3].clip.shape.unwrap();
        assert_eq!(shape.radius, 0.0);
        assert!(q[3].clip.rect.w > 40.0);
    }

    #[test]
    fn layers_wrap_their_subtree_and_ignore_inner_opacity() {
        let mut s = Scene::new(Fonts::new());
        let root = s.root();
        let group = s.add(root, Node::item().x(5.0).y(6.0).size(20.0, 10.0).opacity(0.5).layer(true));
        s.add(group, Node::rect().size(20.0, 10.0).color(Color::WHITE));
        s.add(root, Node::rect().x(60.0).size(4.0, 4.0).color(Color::WHITE));
        s.set_viewport(Vec2::new(100.0, 50.0), 1.0);
        let primitives = s.frame().primitives.to_vec();
        let Primitive::LayerBegin(begin) = primitives[0] else { panic!("{:?}", primitives[0]) };
        assert_eq!(begin.opacity, 0.5);
        assert_eq!(begin.bounds, Rect::new(3.0, 4.0, 24.0, 14.0));
        let Primitive::Quad(inner) = primitives[1] else { panic!() };
        assert_eq!(inner.start.a, 1.0);
        assert_eq!(primitives[2], Primitive::LayerEnd);
        let Primitive::Quad(outside) = primitives[3] else { panic!() };
        assert_eq!(outside.start.a, 1.0);
    }

    #[test]
    fn empty_layers_emit_nothing() {
        let mut s = Scene::new(Fonts::new());
        let root = s.root();
        s.add(root, Node::item().size(20.0, 10.0).layer(true));
        s.set_viewport(Vec2::new(100.0, 50.0), 1.0);
        assert!(s.frame().primitives.is_empty());
    }

    #[test]
    fn layer_effects_grow_the_bounds_in_device_pixels() {
        let mut s = Scene::new(Fonts::new());
        let root = s.root();
        let effect = LayerEffect::blur(0.25);
        let group = s.add(root, Node::item().x(40.0).y(30.0).size(20.0, 10.0).layer_effect(effect));
        s.add(group, Node::rect().size(20.0, 10.0).color(Color::WHITE));
        s.set_viewport(Vec2::new(200.0, 100.0), 2.0);
        let primitives = s.frame().primitives.to_vec();
        let Primitive::LayerBegin(begin) = primitives[0] else { panic!() };
        assert_eq!(begin.fx.blur, 16.0);
        assert_eq!(begin.bounds, Rect::new(61.0, 41.0, 78.0, 58.0));
    }

    fn flick_scene() -> (Scene, crate::Flickable, crate::FlickableNodes) {
        let mut s = scene(100.0, 100.0);
        let root = s.root();
        let flick = crate::Flickable::new().content_size(100.0, 1000.0);
        let nodes = s.flickable(root, Node::item().size(100.0, 100.0), &flick);
        s.frame();
        (s, flick, nodes)
    }

    #[test]
    fn flickable_content_moves_opposite_to_the_offset() {
        let (mut s, flick, nodes) = flick_scene();
        assert_eq!(s.geometry(nodes.content), Some(Rect::new(0.0, 0.0, 100.0, 1000.0)));
        flick.content_y.set(120.0);
        s.frame();
        assert_eq!(s.geometry(nodes.content), Some(Rect::new(0.0, -120.0, 100.0, 1000.0)));
    }

    #[test]
    fn dragging_scrolls_after_the_start_distance_and_cancels_the_click() {
        let (mut s, flick, nodes) = flick_scene();
        let clicks = Rc::new(Cell::new(0));
        let seen = clicks.clone();
        s.add(
            nodes.content,
            Node::rect().size(100.0, 1000.0).mouse(MouseArea::new().on_click(move |_| seen.set(seen.get() + 1))),
        );
        s.frame();
        s.pointer_motion_at(Vec2::new(50.0, 80.0), 0.0);
        s.pointer_button_at(MouseButton::Left, true, 0.0);
        s.pointer_motion_at(Vec2::new(50.0, 72.0), 0.01);
        assert_eq!(flick.content_y.get_untracked(), 0.0);
        assert!(!flick.dragging.get_untracked());
        s.pointer_motion_at(Vec2::new(50.0, 60.0), 0.02);
        assert!(flick.dragging.get_untracked());
        assert_eq!(flick.content_y.get_untracked(), 0.0);
        s.pointer_motion_at(Vec2::new(50.0, 40.0), 0.03);
        assert!((flick.content_y.get_untracked() - 20.0).abs() < 1e-3);
        s.pointer_motion_at(Vec2::new(50.0, 40.0), 0.5);
        s.pointer_button_at(MouseButton::Left, false, 0.5);
        assert_eq!(clicks.get(), 0);
        assert!(!flick.dragging.get_untracked());
        assert!(!flick.flicking.get_untracked());
    }

    #[test]
    fn a_tap_without_movement_still_clicks_the_child() {
        let (mut s, flick, nodes) = flick_scene();
        let clicks = Rc::new(Cell::new(0));
        let seen = clicks.clone();
        s.add(
            nodes.content,
            Node::rect().size(100.0, 1000.0).mouse(MouseArea::new().on_click(move |_| seen.set(seen.get() + 1))),
        );
        s.frame();
        s.pointer_motion_at(Vec2::new(50.0, 80.0), 0.0);
        s.pointer_button_at(MouseButton::Left, true, 0.0);
        s.pointer_motion_at(Vec2::new(50.0, 76.0), 0.05);
        s.pointer_button_at(MouseButton::Left, false, 0.1);
        assert_eq!(clicks.get(), 1);
        assert_eq!(flick.content_y.get_untracked(), 0.0);
    }

    #[test]
    fn releasing_a_fast_drag_flicks_and_settles() {
        let (mut s, flick, _nodes) = flick_scene();
        s.pointer_motion_at(Vec2::new(50.0, 90.0), 0.0);
        s.pointer_button_at(MouseButton::Left, true, 0.0);
        s.pointer_motion_at(Vec2::new(50.0, 70.0), 0.01);
        s.pointer_motion_at(Vec2::new(50.0, 50.0), 0.06);
        s.pointer_motion_at(Vec2::new(50.0, 30.0), 0.11);
        let at_release = flick.content_y.get_untracked();
        s.pointer_button_at(MouseButton::Left, false, 0.11);
        assert!(flick.flicking.get_untracked());
        assert!(flick.moving.get_untracked());
        acanthus_anim::tick(0.1);
        let early = flick.content_y.get_untracked();
        assert!(early > at_release);
        for _ in 0..200 {
            acanthus_anim::tick(0.05);
        }
        let settled = flick.content_y.get_untracked();
        assert!(settled > early);
        assert!(!flick.flicking.get_untracked());
        assert!(!flick.moving.get_untracked());
        let expected = at_release + 400.0_f32.powi(2) / (2.0 * crate::flick::FLICK_DECELERATION);
        assert!((settled - expected).abs() < 1.0, "{settled} vs {expected}");
    }

    #[test]
    fn dragging_past_the_end_rubber_bands_and_returns() {
        let (mut s, flick, _nodes) = flick_scene();
        flick.content_y.set(900.0);
        s.pointer_motion_at(Vec2::new(50.0, 90.0), 0.0);
        s.pointer_button_at(MouseButton::Left, true, 0.0);
        s.pointer_motion_at(Vec2::new(50.0, 60.0), 0.5);
        s.pointer_motion_at(Vec2::new(50.0, 20.0), 1.0);
        assert!(flick.content_y.get_untracked() > 900.0);
        s.pointer_button_at(MouseButton::Left, false, 2.0);
        for _ in 0..40 {
            acanthus_anim::tick(0.02);
        }
        assert_eq!(flick.content_y.get_untracked(), 900.0);
    }

    #[test]
    fn stop_at_bounds_never_leaves_the_content() {
        let (mut s, flick, _nodes) = flick_scene();
        flick.bounds.set(crate::BoundsBehavior::StopAtBounds);
        s.pointer_motion_at(Vec2::new(50.0, 20.0), 0.0);
        s.pointer_button_at(MouseButton::Left, true, 0.0);
        s.pointer_motion_at(Vec2::new(50.0, 50.0), 0.5);
        s.pointer_motion_at(Vec2::new(50.0, 90.0), 1.0);
        assert_eq!(flick.content_y.get_untracked(), 0.0);
        s.pointer_button_at(MouseButton::Left, false, 2.0);
        assert_eq!(flick.content_y.get_untracked(), 0.0);
    }

    #[test]
    fn wheel_scrolls_the_flickable_and_clamps() {
        let (mut s, flick, _nodes) = flick_scene();
        s.pointer_motion_at(Vec2::new(50.0, 50.0), 0.0);
        s.scroll(ScrollDelta { x: 0.0, y: 40.0 });
        assert_eq!(flick.content_y.get_untracked(), 40.0);
        s.scroll(ScrollDelta { x: 0.0, y: 5000.0 });
        assert_eq!(flick.content_y.get_untracked(), 900.0);
        s.scroll(ScrollDelta { x: 0.0, y: -5000.0 });
        assert_eq!(flick.content_y.get_untracked(), 0.0);
    }

    fn list_scene(count: usize, cache: f32) -> (Scene, crate::ListView, ListSignal<u32>) {
        let mut s = scene(100.0, 100.0);
        let root = s.root();
        let model = ListSignal::new((0..count as u32).collect::<Vec<_>>());
        let view = s.list_view(
            root,
            Node::item().size(100.0, 100.0),
            crate::ListViewConfig::default().spacing(4.0).cache_buffer(cache),
            &model,
            |v| *v,
            |scene, parent, item| scene.add(parent, Node::rect().size(100.0, 20.0)).tap_index(item),
        );
        s.frame();
        (s, view, model)
    }

    trait TapIndex {
        fn tap_index<T>(self, item: crate::Item<T>) -> Self;
    }

    impl TapIndex for NodeId {
        fn tap_index<T>(self, _item: crate::Item<T>) -> Self {
            self
        }
    }

    #[test]
    fn list_view_only_instantiates_the_visible_range() {
        let (mut s, view, _model) = list_scene(1000, 0.0);
        assert_eq!(view.count.get_untracked(), 1000);
        assert_eq!(view.flickable.content_height.get_untracked(), 1000.0 * 24.0 - 4.0);
        assert_eq!(view.instantiated.get_untracked(), 5);
        view.flickable.content_y.set(12_000.0);
        s.frame();
        assert_eq!(view.instantiated.get_untracked(), 5);
        let content = s.get(view.nodes.content).unwrap().children().to_vec();
        let ys: Vec<f32> = content.iter().map(|c| s.geometry(*c).unwrap().y).collect();
        assert_eq!(ys, vec![0.0, 24.0, 48.0, 72.0, 96.0]);
        assert_eq!(s.get(content[0]).unwrap().y.get_untracked(), 12_000.0);
    }

    #[test]
    fn list_view_cache_buffer_keeps_extra_items_alive() {
        let (_s, view, _model) = list_scene(1000, 48.0);
        assert!(view.instantiated.get_untracked() > 5);
        assert!(view.instantiated.get_untracked() < 12);
    }

    #[test]
    fn list_view_follows_model_changes() {
        let (mut s, view, model) = list_scene(3, 0.0);
        assert_eq!(view.instantiated.get_untracked(), 3);
        model.push(3);
        model.push(4);
        model.push(5);
        s.frame();
        assert_eq!(view.count.get_untracked(), 6);
        assert_eq!(view.instantiated.get_untracked(), 5);
        model.clear();
        s.frame();
        assert_eq!(view.count.get_untracked(), 0);
        assert_eq!(view.instantiated.get_untracked(), 0);
        assert_eq!(view.flickable.content_height.get_untracked(), 0.0);
    }

    fn transition_scene() -> (Scene, crate::ListView, ListSignal<u32>) {
        let mut s = scene(100.0, 100.0);
        let root = s.root();
        let model = ListSignal::new(vec![0u32, 1]);
        let transitions = crate::ListTransitions {
            add_scale_from: 0.5,
            add_scale_ms: 100,
            add_opacity_ms: 100,
            remove_opacity_ms: 100,
            easing: acanthus_anim::Easing::Linear,
        };
        let view = s.list_view(
            root,
            Node::item().size(100.0, 100.0),
            crate::ListViewConfig::default().spacing(4.0).transitions(transitions),
            &model,
            |v| *v,
            |scene, parent, _item| scene.add(parent, Node::rect().size(100.0, 20.0)),
        );
        s.frame();
        (s, view, model)
    }

    #[test]
    fn list_view_add_transition_animates_only_new_rows() {
        let (mut s, view, model) = transition_scene();
        let first = s.get(view.nodes.content).unwrap().children().to_vec();
        assert_eq!(s.get(first[0]).unwrap().opacity.get_untracked(), 1.0);
        model.push(2);
        s.frame();
        let rows = s.get(view.nodes.content).unwrap().children().to_vec();
        let fresh = s.get(rows[2]).unwrap();
        assert_eq!(fresh.opacity.get_untracked(), 0.0);
        assert_eq!(fresh.scale.get_untracked(), 0.5);
        assert_eq!(s.get(rows[0]).unwrap().opacity.get_untracked(), 1.0);
        acanthus_anim::tick(0.05);
        let fresh = s.get(rows[2]).unwrap();
        assert!((fresh.opacity.get_untracked() - 0.5).abs() < 1e-3);
        assert!((fresh.scale.get_untracked() - 0.75).abs() < 1e-3);
        acanthus_anim::tick(0.1);
        let fresh = s.get(rows[2]).unwrap();
        assert_eq!(fresh.opacity.get_untracked(), 1.0);
        assert_eq!(fresh.scale.get_untracked(), 1.0);
    }

    #[test]
    fn list_view_remove_transition_fades_then_drops_the_row() {
        let (mut s, view, model) = transition_scene();
        let rows = s.get(view.nodes.content).unwrap().children().to_vec();
        model.replace(vec![1]);
        s.frame();
        assert!(s.get(rows[0]).is_some());
        assert_eq!(view.count.get_untracked(), 1);
        acanthus_anim::tick(0.05);
        assert!((s.get(rows[0]).unwrap().opacity.get_untracked() - 0.5).abs() < 1e-3);
        acanthus_anim::tick(0.1);
        s.frame();
        assert!(s.get(rows[0]).is_none());
        assert!(s.get(rows[1]).is_some());
    }

    #[test]
    fn list_view_positions_the_view_at_an_index() {
        let (mut s, view, _model) = list_scene(100, 0.0);
        view.position_view_at_index(50, crate::ViewPosition::Beginning);
        assert_eq!(view.flickable.content_y.get_untracked(), 1200.0);
        view.position_view_at_index(50, crate::ViewPosition::End);
        assert_eq!(view.flickable.content_y.get_untracked(), 1200.0 + 20.0 - 100.0);
        view.position_view_at_index(50, crate::ViewPosition::Center);
        assert_eq!(view.flickable.content_y.get_untracked(), 1200.0 + 10.0 - 50.0);
        view.position_view_at_index(0, crate::ViewPosition::Contain);
        assert_eq!(view.flickable.content_y.get_untracked(), 0.0);
        s.frame();
        assert!(view.instantiated.get_untracked() <= 5);
    }
}
