use std::cell::Cell;
use std::rc::Rc;

use acanthus_anim::Keep;
use acanthus_core::{Effect, ListSignal, Signal};

use crate::model::{Note, pinned_first};

#[derive(Clone)]
pub struct Controller {
    pub notes: ListSignal<Note>,
    pub selected: Signal<Option<u32>>,
    pub text: Signal<String>,
    next: Rc<Cell<u32>>,
}

impl Controller {
    pub fn new(keep: &mut Keep) -> Self {
        let controller = Controller {
            notes: ListSignal::default(),
            selected: Signal::new(None),
            text: Signal::new(String::new()),
            next: Rc::new(Cell::new(1)),
        };
        keep.add(Effect::new({
            let controller = controller.clone();
            move || {
                let id = controller.selected.get();
                let text = id.and_then(|id| controller.find(id)).map(|note| note.text).unwrap_or_default();
                controller.text.set(text);
            }
        }));
        keep.add(Effect::new({
            let controller = controller.clone();
            move || {
                let text = controller.text.get();
                controller.edit(text);
            }
        }));
        controller
    }

    fn find(&self, id: u32) -> Option<Note> {
        self.notes.get_untracked().into_iter().find(|note| note.id == id)
    }

    fn position(&self, id: u32) -> Option<usize> {
        self.notes.get_untracked().iter().position(|note| note.id == id)
    }

    pub fn current(&self) -> Option<Note> {
        self.selected.get().and_then(|id| self.notes.with(|notes| notes.iter().find(|n| n.id == id).cloned()))
    }

    pub fn add(&self) {
        let id = self.next.get();
        self.next.set(id + 1);
        self.notes.insert(0, Note { id, text: String::new(), pinned: false });
        self.notes.replace(pinned_first(self.notes.get_untracked()));
        self.selected.set(Some(id));
    }

    pub fn delete(&self) {
        let Some(index) = self.selected.get_untracked().and_then(|id| self.position(id)) else { return };
        self.notes.remove(index);
        let next = self.notes.at(index.min(self.notes.len().saturating_sub(1))).map(|note| note.id);
        self.selected.set(next);
    }

    pub fn set_pinned(&self, pinned: bool) {
        let Some(index) = self.selected.get_untracked().and_then(|id| self.position(id)) else { return };
        self.notes.update_at(index, |note| note.pinned = pinned);
        self.notes.replace(pinned_first(self.notes.get_untracked()));
    }

    fn edit(&self, text: String) {
        let Some(index) = self.selected.get_untracked().and_then(|id| self.position(id)) else { return };
        if self.notes.get_untracked()[index].text != text {
            self.notes.update_at(index, |note| note.text = text);
        }
    }
}
