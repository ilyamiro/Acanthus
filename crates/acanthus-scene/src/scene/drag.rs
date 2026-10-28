use std::rc::Rc;

use acanthus_core::Vec2;

use super::Scene;
use crate::drag::{DragData, DragOffer, DropAction, DropEvent};
use crate::node::NodeId;

pub(super) struct DragSession {
    offer: DragOffer,
    data: Option<Rc<DragData>>,
    over: Option<NodeId>,
}

impl DragSession {
    pub(super) fn is_internal(&self) -> bool {
        self.data.is_some()
    }
}

impl Scene {
    pub(super) fn drag_candidate(&mut self, hits: &[NodeId], pos: Vec2) {
        self.drag_pending =
            hits.iter().rev().copied().find(|id| self.nodes[*id].drag_source.is_some()).map(|id| (id, pos));
    }

    pub(super) fn drag_pointer_moved(&mut self, pos: Vec2) -> bool {
        if self.drag.is_none()
            && let Some((id, start)) = self.drag_pending
        {
            let threshold = self.nodes.get(id).and_then(|n| n.drag_source.as_ref()).map_or(f32::MAX, |s| s.threshold);
            if (pos - start).length() < threshold {
                return false;
            }
            self.drag_pending = None;
            self.start_scene_drag(id);
        }
        if self.drag.is_some() {
            self.update_drop_target(pos);
            return true;
        }
        false
    }

    fn start_scene_drag(&mut self, id: NodeId) {
        let Some(source) = self.nodes.get_mut(id).and_then(|n| n.drag_source.as_mut()) else { return };
        let data = (source.data)();
        let offer = DragOffer { mime_types: data.mime_types(), actions: source.actions, source: Some(id) };
        source.active.set(true);
        self.drag = Some(DragSession { offer, data: Some(Rc::new(data)), over: None });
        if let Some((target, _)) = self.pressed.take()
            && let Some(m) = self.nodes.get(target).and_then(|n| n.mouse.as_ref())
        {
            m.pressed.set(false);
        }
        let now = self.now();
        self.release_flick(now);
    }

    fn drop_target(&self, pos: Vec2, offer: &DragOffer) -> Option<NodeId> {
        self.hits(pos)
            .into_iter()
            .rev()
            .find(|id| self.nodes[*id].drop.as_ref().is_some_and(|area| area.accepts(offer).is_some()))
    }

    fn update_drop_target(&mut self, pos: Vec2) {
        let Some(session) = self.drag.as_ref() else { return };
        let (offer, previous) = (session.offer.clone(), session.over);
        let target = self.drop_target(pos, &offer);
        if target != previous {
            if let Some(area) = previous.and_then(|id| self.nodes.get_mut(id)).and_then(|n| n.drop.as_mut()) {
                area.contains_drag.set(false);
                if let Some(f) = area.on_exited.as_mut() {
                    f();
                }
            }
            if let Some(area) = target.and_then(|id| self.nodes.get_mut(id)).and_then(|n| n.drop.as_mut()) {
                area.contains_drag.set(true);
                if let Some(f) = area.on_entered.as_mut() {
                    f(&offer);
                }
            }
            if let Some(session) = self.drag.as_mut() {
                session.over = target;
            }
        }
        if let Some(id) = target
            && let Some(local) = self.local_point(id, pos)
            && let Some(area) = self.nodes.get_mut(id).and_then(|n| n.drop.as_mut())
        {
            area.drag_position.set(local);
            if let Some(f) = area.on_moved.as_mut() {
                f(local);
            }
        }
    }

    fn end_drag(&mut self, drop: Option<(Option<Rc<DragData>>, Vec2)>) -> Option<DropAction> {
        let session = self.drag.take()?;
        let mut performed = None;
        if let Some(target) = session.over {
            let payload = drop.as_ref().and_then(|(data, _)| data.clone()).or(session.data.clone());
            let local = drop.as_ref().and_then(|(_, pos)| self.local_point(target, *pos)).unwrap_or_default();
            if let Some(area) = self.nodes.get_mut(target).and_then(|n| n.drop.as_mut()) {
                area.contains_drag.set(false);
                let action = area.accepts(&session.offer);
                match (drop.is_some(), payload, action) {
                    (true, Some(data), Some(action)) => {
                        if let Some(f) = area.on_dropped.as_mut() {
                            f(DropEvent { position: local, data, action, source: session.offer.source });
                        }
                        performed = Some(action);
                    }
                    _ => {
                        if let Some(f) = area.on_exited.as_mut() {
                            f();
                        }
                    }
                }
            }
        }
        if let Some(source) =
            session.offer.source.and_then(|id| self.nodes.get_mut(id)).and_then(|n| n.drag_source.as_mut())
        {
            source.active.set(false);
            if let Some(f) = source.on_finished.as_mut() {
                f(performed);
            }
        }
        performed
    }

    pub(super) fn drag_pointer_released(&mut self, pos: Vec2) -> bool {
        self.drag_pending = None;
        if self.drag.is_none() {
            return false;
        }
        self.end_drag(Some((None, pos)));
        true
    }

    /// True while a drag is running over the scene, started inside it or by another application.
    pub fn drag_active(&self) -> bool {
        self.drag.is_some()
    }

    /// Cancels a running drag: the area under it sees it leave and the source is told nothing was performed.
    pub fn drag_cancel(&mut self) {
        self.drag_pending = None;
        self.end_drag(None);
    }

    /// A drag from another application enters the scene at `pos` (compositor logical pixels). Returns whether a [`crate::DropArea`] accepts it there.
    pub fn drag_enter(&mut self, offer: DragOffer, pos: Vec2) -> bool {
        self.drag_cancel();
        self.drag = Some(DragSession { offer, data: None, over: None });
        self.drag_motion(pos).is_some()
    }

    /// The external drag moves to `pos`. Returns the action a drop there would perform, `None` where nothing accepts it.
    pub fn drag_motion(&mut self, pos: Vec2) -> Option<DropAction> {
        let pos = pos / self.units;
        self.drag_position = pos;
        self.update_drop_target(pos);
        let session = self.drag.as_ref()?;
        let area = self.nodes.get(session.over?)?.drop.as_ref()?;
        area.accepts(&session.offer)
    }

    /// The external drag left the scene.
    pub fn drag_leave(&mut self) {
        self.drag_cancel();
    }

    /// The external drag was dropped at its last position with `data`. Returns the action performed.
    pub fn drag_drop(&mut self, data: DragData) -> Option<DropAction> {
        let pos = self.drag_position;
        self.end_drag(Some((Some(Rc::new(data)), pos)))
    }
}

#[cfg(test)]
mod tests {
    use std::cell::{Cell, RefCell};

    use super::*;
    use crate::drag::{DragSource, DropActions, DropArea};
    use crate::input::{MouseArea, MouseButton};
    use crate::keyboard::{Key, KeyEvent};
    use crate::{Fonts, Node};

    struct Rig {
        scene: Scene,
        dropped: Rc<RefCell<Vec<(String, DropAction, Vec2)>>>,
        entered: Rc<Cell<u32>>,
        exited: Rc<Cell<u32>>,
        finished: Rc<RefCell<Vec<Option<DropAction>>>>,
        contains: acanthus_core::Signal<bool>,
        active: acanthus_core::Signal<bool>,
        clicks: Rc<Cell<u32>>,
    }

    fn rig(keys: &[&str]) -> Rig {
        let mut scene = Scene::new(Fonts::new());
        scene.set_viewport(Vec2::new(300.0, 100.0), 1.0);
        let root = scene.root();
        let dropped = Rc::new(RefCell::new(Vec::new()));
        let (entered, exited) = (Rc::new(Cell::new(0)), Rc::new(Cell::new(0)));
        let finished = Rc::new(RefCell::new(Vec::new()));
        let clicks = Rc::new(Cell::new(0));

        let (d, e, x) = (dropped.clone(), entered.clone(), exited.clone());
        let area = DropArea::new()
            .keys(keys)
            .on_entered(move |_| e.set(e.get() + 1))
            .on_exited(move || x.set(x.get() + 1))
            .on_dropped(move |event| {
                d.borrow_mut().push((event.data.as_text().unwrap_or_default(), event.action, event.position))
            });
        let contains = area.contains_drag.clone();
        scene.add(root, Node::rect().x(100.0).size(50.0, 50.0).drop_area(area));

        let f = finished.clone();
        let source =
            DragSource::new(|| DragData::text("payload")).on_finished(move |action| f.borrow_mut().push(action));
        let active = source.active.clone();
        let c = clicks.clone();
        scene.add(
            root,
            Node::rect()
                .size(50.0, 50.0)
                .mouse(MouseArea::new().on_click(move |_| c.set(c.get() + 1)))
                .drag_source(source),
        );
        scene.frame();
        Rig { scene, dropped, entered, exited, finished, contains, active, clicks }
    }

    fn press_and_drag(rig: &mut Rig, to: Vec2) {
        rig.scene.pointer_motion(Vec2::new(10.0, 10.0));
        rig.scene.pointer_button(MouseButton::Left, true);
        rig.scene.pointer_motion(to);
    }

    #[test]
    fn a_drag_starts_after_the_threshold_and_drops_on_an_area() {
        let mut rig = rig(&[]);
        press_and_drag(&mut rig, Vec2::new(14.0, 10.0));
        assert!(!rig.scene.drag_active());
        assert!(!rig.active.get_untracked());

        rig.scene.pointer_motion(Vec2::new(120.0, 20.0));
        assert!(rig.scene.drag_active());
        assert!(rig.active.get_untracked());
        assert!(rig.contains.get_untracked());
        assert_eq!(rig.entered.get(), 1);

        rig.scene.pointer_button(MouseButton::Left, false);
        assert_eq!(rig.dropped.borrow().as_slice(), [("payload".to_owned(), DropAction::Copy, Vec2::new(20.0, 20.0))]);
        assert_eq!(rig.finished.borrow().as_slice(), [Some(DropAction::Copy)]);
        assert!(!rig.scene.drag_active());
        assert!(!rig.active.get_untracked());
        assert!(!rig.contains.get_untracked());
        assert_eq!(rig.clicks.get(), 0);
    }

    #[test]
    fn leaving_the_area_and_releasing_elsewhere_drops_nothing() {
        let mut rig = rig(&[]);
        press_and_drag(&mut rig, Vec2::new(120.0, 20.0));
        rig.scene.pointer_motion(Vec2::new(250.0, 20.0));
        assert!(!rig.contains.get_untracked());
        assert_eq!(rig.exited.get(), 1);
        rig.scene.pointer_button(MouseButton::Left, false);
        assert!(rig.dropped.borrow().is_empty());
        assert_eq!(rig.finished.borrow().as_slice(), [None]);
    }

    #[test]
    fn escape_cancels_a_drag() {
        let mut rig = rig(&[]);
        press_and_drag(&mut rig, Vec2::new(120.0, 20.0));
        assert!(rig.scene.key_event(&KeyEvent::new(Key::Escape)));
        assert!(!rig.scene.drag_active());
        assert!(!rig.contains.get_untracked());
        assert_eq!(rig.exited.get(), 1);
        assert_eq!(rig.finished.borrow().as_slice(), [None]);
        rig.scene.pointer_button(MouseButton::Left, false);
        assert!(rig.dropped.borrow().is_empty());
    }

    #[test]
    fn an_area_with_other_keys_does_not_take_the_drag() {
        let mut rig = rig(&["image/png"]);
        press_and_drag(&mut rig, Vec2::new(120.0, 20.0));
        assert!(rig.scene.drag_active());
        assert!(!rig.contains.get_untracked());
        rig.scene.pointer_button(MouseButton::Left, false);
        assert!(rig.dropped.borrow().is_empty());
        assert_eq!(rig.finished.borrow().as_slice(), [None]);
    }

    #[test]
    fn a_plain_click_still_clicks_a_node_that_can_start_drags() {
        let mut rig = rig(&[]);
        rig.scene.pointer_motion(Vec2::new(10.0, 10.0));
        rig.scene.pointer_button(MouseButton::Left, true);
        rig.scene.pointer_button(MouseButton::Left, false);
        assert_eq!(rig.clicks.get(), 1);
        assert!(rig.finished.borrow().is_empty());
    }

    #[test]
    fn external_drags_enter_move_and_drop_with_their_data() {
        let mut rig = rig(&["text/uri-list"]);
        let offer = DragOffer { mime_types: vec!["text/uri-list".into()], actions: DropActions::COPY, source: None };
        assert!(!rig.scene.drag_enter(offer.clone(), Vec2::new(250.0, 20.0)));
        assert_eq!(rig.scene.drag_motion(Vec2::new(110.0, 30.0)), Some(DropAction::Copy));
        assert!(rig.contains.get_untracked());
        assert_eq!(rig.entered.get(), 1);
        let data = DragData::uri_list(&["file:///tmp/a.png"]).with("text/plain", &b"file:///tmp/a.png"[..]);
        assert_eq!(rig.scene.drag_drop(data), Some(DropAction::Copy));
        assert_eq!(rig.dropped.borrow().len(), 1);
        assert_eq!(rig.dropped.borrow()[0].1, DropAction::Copy);
        assert!(!rig.scene.drag_active());

        assert!(rig.scene.drag_enter(offer, Vec2::new(110.0, 30.0)));
        rig.scene.drag_leave();
        assert_eq!(rig.exited.get(), 1);
        assert!(!rig.contains.get_untracked());
        assert_eq!(rig.scene.drag_drop(DragData::text("late")), None);
    }

    #[test]
    fn the_topmost_accepting_area_wins() {
        let mut scene = Scene::new(Fonts::new());
        scene.set_viewport(Vec2::new(100.0, 100.0), 1.0);
        let root = scene.root();
        let lower = DropArea::new();
        let lower_contains = lower.contains_drag.clone();
        scene.add(root, Node::rect().size(100.0, 100.0).drop_area(lower));
        let picky = DropArea::new().keys(&["image/png"]);
        let picky_contains = picky.contains_drag.clone();
        scene.add(root, Node::rect().size(100.0, 100.0).drop_area(picky));
        scene.frame();
        let offer = DragOffer { mime_types: vec!["text/plain".into()], actions: DropActions::ALL, source: None };
        assert!(scene.drag_enter(offer, Vec2::new(50.0, 50.0)));
        assert!(lower_contains.get_untracked());
        assert!(!picky_contains.get_untracked());
    }
}
