use std::rc::Rc;

use acanthus_io::{IoError, IpcRegistry};
use acanthus_services::platform::compositor::CompositorKind;
use acanthus_services::platform::dbus::BusKind;
use acanthus_services::platform::shortcuts::{ChordError, Launcher, Shortcut, Shortcuts, ShortcutsConfig, Strategy};
use calloop::LoopHandle;

use crate::actions::{Actions, Verb};

struct Binding {
    shortcut: Shortcut,
    verb: Verb,
    target: &'static str,
    extra: &'static str,
}

/// Key chords mapped onto [`Actions`]: the one table behind compositor bindings, the GlobalShortcuts portal and the IPC `activate` call, so a key press, a command line and a markup trigger run the same action.
#[derive(Default)]
pub struct Keybindings {
    bindings: Vec<Binding>,
}

impl Keybindings {
    /// An empty table.
    pub fn new() -> Keybindings {
        Keybindings::default()
    }

    /// Binds `chord` to `verb target extra`. `id` names the binding to the compositor and the portal; `description` is shown in the desktop's shortcut dialog.
    pub fn bind(
        mut self,
        id: &str,
        description: &str,
        chord: &str,
        verb: Verb,
        target: &'static str,
        extra: &'static str,
    ) -> Result<Keybindings, ChordError> {
        self.bindings.push(Binding { shortcut: Shortcut::new(id, description, chord)?, verb, target, extra });
        Ok(self)
    }

    /// The shortcuts to register.
    pub fn shortcuts(&self) -> Vec<Shortcut> {
        self.bindings.iter().map(|b| b.shortcut.clone()).collect()
    }

    /// Runs the action bound to `id`; false when no binding has that id or no action handled it.
    pub fn activate(&self, actions: &Actions, id: &str) -> bool {
        self.bindings
            .iter()
            .find(|b| b.shortcut.id == id)
            .is_some_and(|b| actions.dispatch(b.verb.word(), b.target, b.extra))
    }

    /// Exposes `target.method(id)` over IPC; the command compositor bindings run (see [`Launcher`]) calls it.
    pub fn serve(self: &Rc<Self>, actions: &Rc<Actions>, ipc: &IpcRegistry, target: &str, method: &str) {
        let (keys, actions) = (self.clone(), actions.clone());
        ipc.register(target, method, move |args| {
            let id = args.first().and_then(|v| v.as_str()).unwrap_or("");
            keys.activate(&actions, id);
            Ok(serde_json::Value::Null)
        });
    }

    /// Registers the table with the compositor or the portal; portal activations run their action directly, compositor bindings run `launcher`.
    pub fn start<D: 'static>(
        self: &Rc<Self>,
        handle: &LoopHandle<'static, D>,
        actions: &Rc<Actions>,
        launcher: Launcher,
        kind: CompositorKind,
        strategy: Strategy,
        bus: BusKind,
    ) -> Result<Shortcuts, IoError> {
        let config = ShortcutsConfig { shortcuts: self.shortcuts(), launcher, kind, strategy, bus };
        let (keys, actions) = (self.clone(), actions.clone());
        Shortcuts::start(handle, config, move |id| {
            keys.activate(&actions, id);
        })
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;

    use super::*;

    type Seen = Rc<RefCell<Vec<(Verb, String)>>>;

    fn fixture() -> (Rc<Actions>, Rc<Keybindings>, Seen) {
        let actions = Actions::new("stack");
        let seen = Rc::new(RefCell::new(Vec::new()));
        let sink = seen.clone();
        actions.register(&["launcher"], None, move |verb, arg| sink.borrow_mut().push((verb, arg.to_owned())), || {});
        let keys = Rc::new(
            Keybindings::new()
                .bind("toggle-launcher", "Toggle the launcher", "Super+Space", Verb::Toggle, "launcher", "")
                .unwrap()
                .bind("open-apps", "Open applications", "Super+A", Verb::Open, "launcher", "apps")
                .unwrap(),
        );
        (actions, keys, seen)
    }

    #[test]
    fn activation_runs_the_bound_verb_target_and_argument() {
        let (actions, keys, seen) = fixture();
        assert!(keys.activate(&actions, "toggle-launcher"));
        assert!(keys.activate(&actions, "open-apps"));
        assert!(!keys.activate(&actions, "unknown"));
        assert_eq!(*seen.borrow(), [(Verb::Toggle, String::new()), (Verb::Open, "apps".to_owned())]);
    }

    #[test]
    fn an_unparsable_chord_is_an_error() {
        assert!(Keybindings::new().bind("x", "X", "Super+", Verb::Open, "launcher", "").is_err());
    }

    #[test]
    fn ipc_activate_reaches_the_same_action() {
        let (actions, keys, seen) = fixture();
        let ipc = IpcRegistry::new();
        keys.serve(&actions, &ipc, "keys", "activate");
        ipc.call("keys", "activate", &[serde_json::json!("open-apps")]).unwrap();
        assert_eq!(*seen.borrow(), [(Verb::Open, "apps".to_owned())]);
        assert_eq!(keys.shortcuts().len(), 2);
    }
}
