use crate::reactive::Signal;

/// A [`Signal`] holding a `Vec<T>` with edit methods that notify subscribers once per change.
pub struct ListSignal<T>(Signal<Vec<T>>);

impl<T> Clone for ListSignal<T> {
    fn clone(&self) -> Self {
        ListSignal(self.0.clone())
    }
}

impl<T: 'static> Default for ListSignal<T> {
    fn default() -> Self {
        ListSignal::new(Vec::new())
    }
}

impl<T: 'static> ListSignal<T> {
    /// Creates a list with the given items.
    pub fn new(items: Vec<T>) -> Self {
        ListSignal(Signal::new(items))
    }

    /// The underlying signal, for reading the whole vector inside an effect.
    pub fn signal(&self) -> &Signal<Vec<T>> {
        &self.0
    }

    /// Reads the items as a slice and tracks the list.
    pub fn with<R>(&self, f: impl FnOnce(&[T]) -> R) -> R {
        self.0.with(|items| f(items))
    }

    /// Number of items; tracks the list.
    pub fn len(&self) -> usize {
        self.0.with(Vec::len)
    }

    /// Whether the list is empty; tracks the list.
    pub fn is_empty(&self) -> bool {
        self.0.with(Vec::is_empty)
    }

    /// Replaces all items and always notifies, even when the new items are equal.
    pub fn replace(&self, items: Vec<T>) {
        self.0.set_forced(items);
    }

    /// Appends an item.
    pub fn push(&self, item: T) {
        self.0.update(|items| items.push(item));
    }

    /// Inserts an item at `index`, clamped to the end of the list.
    pub fn insert(&self, index: usize, item: T) {
        self.0.update(|items| items.insert(index.min(items.len()), item));
    }

    /// Removes and returns the item at `index`; `None` and no notification when out of range.
    pub fn remove(&self, index: usize) -> Option<T> {
        if index >= self.0.with_untracked(Vec::len) {
            return None;
        }
        let mut removed = None;
        self.0.update(|items| removed = Some(items.remove(index)));
        removed
    }

    /// Moves the item at `from` to position `to`; does nothing when either index is out of range or they are equal.
    pub fn move_item(&self, from: usize, to: usize) {
        let len = self.0.with_untracked(Vec::len);
        if from >= len || to >= len || from == to {
            return;
        }
        self.0.update(|items| {
            let item = items.remove(from);
            items.insert(to, item);
        });
    }

    /// Edits the item at `index` in place; does nothing when out of range.
    pub fn update_at(&self, index: usize, f: impl FnOnce(&mut T)) {
        if index >= self.0.with_untracked(Vec::len) {
            return;
        }
        self.0.update(|items| f(&mut items[index]));
    }

    /// Removes all items; does nothing (no notification) when already empty.
    pub fn clear(&self) {
        if self.0.with_untracked(Vec::is_empty) {
            return;
        }
        self.0.update(Vec::clear);
    }
}

impl<T: Clone + 'static> ListSignal<T> {
    /// A clone of all items; tracks the list.
    pub fn get(&self) -> Vec<T> {
        self.0.get()
    }

    /// A clone of all items without tracking.
    pub fn get_untracked(&self) -> Vec<T> {
        self.0.get_untracked()
    }

    /// A clone of the item at `index`, if any; tracks the list.
    pub fn at(&self, index: usize) -> Option<T> {
        self.0.with(|items| items.get(index).cloned())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::reactive::Effect;
    use std::cell::Cell;
    use std::rc::Rc;

    #[test]
    fn edits_notify_subscribers() {
        let list = ListSignal::new(vec![1, 2, 3]);
        let runs = Rc::new(Cell::new(0));
        let effect = {
            let list = list.clone();
            let runs = runs.clone();
            Effect::new(move || {
                list.len();
                runs.set(runs.get() + 1);
            })
        };
        assert_eq!(runs.get(), 1);
        list.push(4);
        list.insert(0, 0);
        assert_eq!(list.remove(1), Some(1));
        list.move_item(0, 3);
        assert_eq!(list.get(), vec![2, 3, 4, 0]);
        assert_eq!(runs.get(), 5);
        assert_eq!(list.remove(9), None);
        list.move_item(2, 2);
        assert_eq!(runs.get(), 5);
        drop(effect);
    }

    #[test]
    fn update_and_clear() {
        let list = ListSignal::new(vec![1, 2]);
        list.update_at(1, |v| *v = 20);
        assert_eq!(list.at(1), Some(20));
        list.update_at(5, |v| *v = 0);
        list.clear();
        assert!(list.is_empty());
        list.replace(vec![7]);
        assert_eq!(list.get_untracked(), vec![7]);
    }
}
