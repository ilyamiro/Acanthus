use std::cell::RefCell;
use std::rc::Rc;
use std::str::FromStr;

use acanthus_io::IpcRegistry;

use crate::popup::Popup;

/// What a command asks a popup to do.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verb {
    Toggle,
    Open,
    Close,
}

impl Verb {
    /// The command word of the verb (`toggle`, `open`, `close`).
    pub fn word(self) -> &'static str {
        match self {
            Verb::Toggle => "toggle",
            Verb::Open => "open",
            Verb::Close => "close",
        }
    }

    /// Parses a command word; `None` when it is not a verb.
    pub fn parse(word: &str) -> Option<Verb> {
        match word {
            "toggle" => Some(Verb::Toggle),
            "open" => Some(Verb::Open),
            "close" => Some(Verb::Close),
            _ => None,
        }
    }
}

type Run = Box<dyn Fn(Verb, &str)>;
type Observer = Box<dyn Fn(&str, &str, &str)>;

struct Entry {
    names: &'static [&'static str],
    group: Option<&'static str>,
    run: Run,
    close: Box<dyn Fn()>,
    is_open: Option<Box<dyn Fn() -> bool>>,
}

/// Registry of named actions shared by IPC commands, keybindings and UI triggers.
///
/// An action answers to one or more names, runs with a [`Verb`] and a free-form argument, and may
/// belong to an exclusive group: opening one member closes the others. A bare `close` closes the
/// dismiss group and runs the dismiss hooks.
pub struct Actions {
    dismiss_group: &'static str,
    entries: RefCell<Vec<Entry>>,
    dismissals: RefCell<Vec<Rc<dyn Fn()>>>,
    observers: RefCell<Vec<Observer>>,
}

impl Actions {
    /// Creates a registry whose bare `close` command closes every action of `dismiss_group`.
    pub fn new(dismiss_group: &'static str) -> Rc<Actions> {
        Rc::new(Actions {
            dismiss_group,
            entries: RefCell::default(),
            dismissals: RefCell::default(),
            observers: RefCell::default(),
        })
    }

    /// Registers an action named `names` (the first is canonical, the rest are aliases).
    /// `run` handles a verb with its argument, `close` closes the action when its group is dismissed.
    pub fn register(
        &self,
        names: &'static [&'static str],
        group: Option<&'static str>,
        run: impl Fn(Verb, &str) + 'static,
        close: impl Fn() + 'static,
    ) {
        self.entries.borrow_mut().push(Entry {
            names,
            group,
            run: Box::new(run),
            close: Box::new(close),
            is_open: None,
        });
    }

    /// Like [`Actions::register`] with a typed argument: an empty argument arrives as `None`, a value that does not
    /// parse as `A` is logged and the action is not run.
    pub fn register_typed<A: FromStr>(
        &self,
        names: &'static [&'static str],
        group: Option<&'static str>,
        run: impl Fn(Verb, Option<A>) + 'static,
        close: impl Fn() + 'static,
    ) {
        let label = names[0];
        self.register(
            names,
            group,
            move |verb, raw| {
                if raw.is_empty() {
                    return run(verb, None);
                }
                match raw.parse::<A>() {
                    Ok(value) => run(verb, Some(value)),
                    Err(_) => tracing::warn!("{label}: argument `{raw}` rejected"),
                }
            },
            close,
        );
    }

    /// Registers a [`Popup`] under its names and group. Its open state takes part in [`Actions::escape`].
    pub fn add(&self, popup: Rc<dyn Popup>) {
        let (run, close, is_open) = (popup.clone(), popup.clone(), popup.clone());
        self.entries.borrow_mut().push(Entry {
            names: popup.names(),
            group: popup.group(),
            run: Box::new(move |verb, arg| match verb {
                Verb::Toggle => run.toggle(arg),
                Verb::Open => run.open(arg),
                Verb::Close => run.close(),
            }),
            close: Box::new(move || close.close()),
            is_open: Some(Box::new(move || is_open.is_open())),
        });
    }

    /// Canonical names of the registered popups that are open right now.
    pub fn open_popups(&self) -> Vec<&'static str> {
        let entries = self.entries.borrow();
        entries.iter().filter(|e| e.is_open.as_ref().is_some_and(|open| open())).map(|e| e.names[0]).collect()
    }

    /// Escape: closes every open popup. Returns whether any was open.
    pub fn escape(&self) -> bool {
        let open: Vec<_> = {
            let entries = self.entries.borrow();
            (0..entries.len()).filter(|&i| entries[i].is_open.as_ref().is_some_and(|open| open())).collect()
        };
        let entries = self.entries.borrow();
        for &index in &open {
            (entries[index].close)();
        }
        !open.is_empty()
    }

    /// Registers a name that is recognised but not implemented yet; it only logs the request.
    pub fn pending(&self, names: &'static [&'static str], group: Option<&'static str>) {
        let label = names[0];
        self.register(
            names,
            group,
            move |verb, extra| tracing::info!("{label}: {} requested, not ported yet ({extra})", verb.word()),
            || {},
        );
    }

    /// Adds a hook that runs on every bare `close`, after the dismiss group is closed.
    pub fn on_dismiss(&self, hide: impl Fn() + 'static) {
        self.dismissals.borrow_mut().push(Rc::new(hide));
    }

    /// Adds an observer called with `(command, target, extra)` before every dispatch.
    pub fn on_dispatch(&self, observer: impl Fn(&str, &str, &str) + 'static) {
        self.observers.borrow_mut().push(Box::new(observer));
    }

    /// Closes the dismiss group and runs the dismiss hooks.
    pub fn dismiss(&self) {
        self.close_group(self.dismiss_group);
        let hooks: Vec<Rc<dyn Fn()>> = self.dismissals.borrow().clone();
        for hook in hooks {
            hook();
        }
    }

    /// Closes every action registered in `group`.
    pub fn close_group(&self, group: &str) {
        for entry in self.entries.borrow().iter().filter(|entry| entry.group == Some(group)) {
            (entry.close)();
        }
    }

    /// A callback that dispatches `verb` to `name`.
    pub fn trigger(self: &Rc<Self>, verb: Verb, name: &'static str) -> Rc<dyn Fn()> {
        self.trigger_with(verb, name, "")
    }

    /// A callback that dispatches `verb` to `name` with the argument `extra`.
    pub fn trigger_with(self: &Rc<Self>, verb: Verb, name: &'static str, extra: &'static str) -> Rc<dyn Fn()> {
        let actions = self.clone();
        Rc::new(move || {
            actions.dispatch(verb.word(), name, extra);
        })
    }

    /// Runs a command. `command` is a verb (then `target` names the action) or a bare action name
    /// that toggles. Opening a grouped action closes the other members of its group. Returns whether
    /// any action handled it.
    pub fn dispatch(&self, command: &str, target: &str, extra: &str) -> bool {
        for observer in self.observers.borrow().iter() {
            observer(command, target, extra);
        }
        let verb = Verb::parse(command);
        if verb == Some(Verb::Close) && target.is_empty() {
            self.dismiss();
            return true;
        }
        let named = |entry: &Entry| match verb {
            Some(_) => entry.names.contains(&target),
            None => entry.names.contains(&command) || entry.names.contains(&target),
        };
        let entries = self.entries.borrow();
        let mut handled = false;
        for entry in entries.iter().filter(|entry| named(entry)) {
            let verb = verb.unwrap_or(Verb::Toggle);
            if verb != Verb::Close
                && let Some(group) = entry.group
            {
                for other in entries.iter().filter(|other| other.group == Some(group) && !std::ptr::eq(*other, entry)) {
                    (other.close)();
                }
            }
            (entry.run)(verb, extra);
            handled = true;
        }
        handled
    }

    /// Exposes the registry as the IPC function `target.method(command, target, extra)`.
    pub fn serve(self: &Rc<Self>, ipc: &IpcRegistry, target: &str, method: &str) {
        let actions = self.clone();
        ipc.register(target, method, move |args| {
            let arg = |index: usize| args.get(index).and_then(|v| v.as_str()).unwrap_or("");
            actions.dispatch(arg(0), arg(1), arg(2));
            Ok(serde_json::Value::Null)
        });
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;

    use super::*;

    #[test]
    fn typed_arguments_parse_default_to_none_and_reject_garbage() {
        let actions = Actions::new("stack");
        let seen = Rc::new(RefCell::new(Vec::new()));
        let sink = seen.clone();
        actions.register_typed::<u32>(
            &["level"],
            None,
            move |verb, value| sink.borrow_mut().push((verb, value)),
            || {},
        );
        actions.dispatch("open", "level", "7");
        actions.dispatch("open", "level", "");
        actions.dispatch("open", "level", "seven");
        assert_eq!(*seen.borrow(), vec![(Verb::Open, Some(7)), (Verb::Open, None)]);
    }

    const STACK: &str = "stack";

    fn log() -> Rc<RefCell<Vec<String>>> {
        Rc::new(RefCell::new(Vec::new()))
    }

    fn register(
        actions: &Actions,
        log: &Rc<RefCell<Vec<String>>>,
        name: &'static str,
        names: &'static [&'static str],
        group: Option<&'static str>,
    ) {
        let (run_log, close_log) = (log.clone(), log.clone());
        actions.register(
            names,
            group,
            move |verb, extra| run_log.borrow_mut().push(format!("{name}:{verb:?}:{extra}")),
            move || close_log.borrow_mut().push(format!("{name}:closed")),
        );
    }

    #[test]
    fn verb_and_target_select_the_popup() {
        let (actions, log) = (Actions::new(STACK), log());
        register(&actions, &log, "calendar", &["calendar"], None);
        register(&actions, &log, "system", &["system"], None);
        assert!(actions.dispatch("open", "calendar", ""));
        assert!(actions.dispatch("close", "system", ""));
        assert_eq!(*log.borrow(), ["calendar:Open:", "system:Close:"]);
    }

    #[test]
    fn a_trigger_runs_its_verb() {
        let (actions, log) = (Actions::new(STACK), log());
        register(&actions, &log, "launcher", &["launcher"], None);
        actions.trigger(Verb::Toggle, "launcher")();
        actions.trigger(Verb::Close, "launcher")();
        assert_eq!(*log.borrow(), ["launcher:Toggle:", "launcher:Close:"]);
    }

    #[test]
    fn a_trigger_can_carry_an_argument() {
        let (actions, log) = (Actions::new(STACK), log());
        register(&actions, &log, "network", &["network"], None);
        actions.trigger_with(Verb::Toggle, "network", "bt")();
        assert_eq!(*log.borrow(), ["network:Toggle:bt"]);
    }

    #[test]
    fn a_bare_close_dismisses_the_stack_and_runs_the_hooks() {
        let (actions, log) = (Actions::new(STACK), log());
        register(&actions, &log, "calendar", &["calendar"], Some(STACK));
        register(&actions, &log, "music", &["music"], Some(STACK));
        register(&actions, &log, "launcher", &["launcher"], Some("search"));
        let hook_log = log.clone();
        actions.on_dismiss(move || hook_log.borrow_mut().push("menu:hidden".to_owned()));
        assert!(actions.dispatch("close", "", ""));
        assert_eq!(*log.borrow(), ["calendar:closed", "music:closed", "menu:hidden"]);
    }

    #[test]
    fn opening_a_stack_popup_closes_the_other_stack_popups() {
        let (actions, log) = (Actions::new(STACK), log());
        register(&actions, &log, "calendar", &["calendar"], Some(STACK));
        register(&actions, &log, "system", &["system"], Some(STACK));
        actions.dispatch("toggle", "system", "");
        assert_eq!(*log.borrow(), ["calendar:closed", "system:Toggle:"]);
    }

    #[test]
    fn a_pending_target_is_handled_without_effect() {
        let actions = Actions::new(STACK);
        actions.pending(&["settings", "guide"], Some(STACK));
        assert!(actions.dispatch("open", "settings", "general"));
        assert!(actions.dispatch("guide", "", ""));
        assert!(!actions.dispatch("open", "volume", ""));
    }

    #[test]
    fn a_bare_name_toggles() {
        let (actions, log) = (Actions::new(STACK), log());
        register(&actions, &log, "network", &["network"], None);
        assert!(actions.dispatch("network", "", ""));
        assert!(actions.dispatch("network", "wifi", ""));
        assert_eq!(*log.borrow(), ["network:Toggle:", "network:Toggle:"]);
    }

    #[test]
    fn unknown_commands_are_not_handled() {
        let (actions, log) = (Actions::new(STACK), log());
        register(&actions, &log, "calendar", &["calendar"], None);
        assert!(!actions.dispatch("open", "nothing", ""));
        assert!(!actions.dispatch("nothing", "", ""));
        assert!(log.borrow().is_empty());
    }

    #[test]
    fn aliases_reach_the_same_popup() {
        let (actions, log) = (Actions::new(STACK), log());
        register(&actions, &log, "clipboard", &["clipboard", "clip"], None);
        assert!(actions.dispatch("clip", "", ""));
        assert!(actions.dispatch("open", "clip", ""));
        assert_eq!(*log.borrow(), ["clipboard:Toggle:", "clipboard:Open:"]);
    }

    #[test]
    fn opening_closes_the_rest_of_the_group_but_closing_does_not() {
        let (actions, log) = (Actions::new(STACK), log());
        register(&actions, &log, "launcher", &["launcher"], Some("search"));
        register(&actions, &log, "clipboard", &["clipboard"], Some("search"));
        register(&actions, &log, "calendar", &["calendar"], None);
        actions.dispatch("open", "launcher", "");
        actions.dispatch("close", "clipboard", "");
        actions.dispatch("toggle", "calendar", "");
        assert_eq!(*log.borrow(), ["clipboard:closed", "launcher:Open:", "clipboard:Close:", "calendar:Toggle:"]);
    }

    #[test]
    fn the_extra_argument_reaches_the_handler() {
        let (actions, log) = (Actions::new(STACK), log());
        register(&actions, &log, "network", &["network"], None);
        actions.dispatch("open", "network", "bluetooth");
        assert_eq!(*log.borrow(), ["network:Open:bluetooth"]);
    }

    struct Fake {
        names: &'static [&'static str],
        group: Option<&'static str>,
        open: std::cell::Cell<bool>,
    }

    impl Popup for Fake {
        fn names(&self) -> &'static [&'static str] {
            self.names
        }

        fn group(&self) -> Option<&'static str> {
            self.group
        }

        fn is_open(&self) -> bool {
            self.open.get()
        }

        fn open(&self, _: &str) {
            self.open.set(true);
        }

        fn close(&self) {
            self.open.set(false);
        }
    }

    fn fake(names: &'static [&'static str], group: Option<&'static str>) -> Rc<Fake> {
        Rc::new(Fake { names, group, open: std::cell::Cell::new(false) })
    }

    #[test]
    fn popups_toggle_and_exclude_their_group() {
        let actions = Actions::new(STACK);
        let (a, b) = (fake(&["a"], Some(STACK)), fake(&["b", "bee"], Some(STACK)));
        actions.add(a.clone());
        actions.add(b.clone());
        assert!(actions.dispatch("toggle", "a", ""));
        assert!(a.open.get());
        assert!(actions.dispatch("bee", "", ""));
        assert!(b.open.get() && !a.open.get());
        assert!(actions.dispatch("toggle", "bee", ""));
        assert!(!b.open.get());
    }

    #[test]
    fn escape_closes_every_open_popup() {
        let actions = Actions::new(STACK);
        let (a, b) = (fake(&["a"], None), fake(&["b"], None));
        actions.add(a.clone());
        actions.add(b.clone());
        assert!(!actions.escape());
        a.open.set(true);
        b.open.set(true);
        assert_eq!(actions.open_popups(), vec!["a", "b"]);
        assert!(actions.escape());
        assert!(actions.open_popups().is_empty());
    }
}
