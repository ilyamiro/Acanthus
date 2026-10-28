//! The contract of an action that opens and closes a piece of UI.

/// A popup the app can open, close and toggle by name, from IPC, keybindings or Escape.
pub trait Popup {
    /// Names the popup answers to; the first is canonical, the rest are aliases.
    fn names(&self) -> &'static [&'static str];

    /// Exclusive group: opening one member closes the others.
    fn group(&self) -> Option<&'static str> {
        None
    }

    /// Whether the popup is shown.
    fn is_open(&self) -> bool;

    /// Shows the popup; `arg` is the free-form argument of the command (for example a tab name).
    fn open(&self, arg: &str);

    /// Hides the popup.
    fn close(&self);

    /// Opens the popup when closed and closes it when open.
    fn toggle(&self, arg: &str) {
        if self.is_open() {
            self.close();
        } else {
            self.open(arg);
        }
    }
}

/// Wraps a key handler of a popup so that Escape runs `close` and is consumed; every other key goes to `keys`.
pub fn closing_on_escape(
    close: impl Fn() + 'static,
    mut keys: impl FnMut(&acanthus_scene::KeyEvent) -> bool + 'static,
) -> impl FnMut(&acanthus_scene::KeyEvent) -> bool + 'static {
    move |event| {
        if event.is(acanthus_scene::Key::Escape) {
            close();
            true
        } else {
            keys(event)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::closing_on_escape;
    use acanthus_scene::{Key, KeyEvent};
    use std::cell::Cell;
    use std::rc::Rc;

    #[test]
    fn escape_closes_and_other_keys_pass_through() {
        let closed = Rc::new(Cell::new(0));
        let seen = Rc::new(Cell::new(0));
        let (c, s) = (closed.clone(), seen.clone());
        let mut handler = closing_on_escape(
            move || c.set(c.get() + 1),
            move |_| {
                s.set(s.get() + 1);
                false
            },
        );
        assert!(handler(&KeyEvent::new(Key::Escape)));
        assert_eq!((closed.get(), seen.get()), (1, 0));
        assert!(!handler(&KeyEvent::new(Key::Down)));
        assert_eq!((closed.get(), seen.get()), (1, 1));
    }
}
