use std::rc::Rc;

use acanthus_app::{Actions, Verb};

use crate::{Functions, Value};

/// Registers `action(command, target?, extra?)` in `functions`: it dispatches to `actions` exactly as an IPC or keybinding command does (`action("settings")` toggles, `action("open", "settings")` picks a verb) and answers `null`. A literal first argument is checked at compile time: it must be a verb or the name of a registered action.
pub fn bind_actions(functions: &mut Functions, actions: &Rc<Actions>) {
    let known = actions.clone();
    functions.check_literal("action", move |name| {
        (Verb::parse(name).is_none() && !known.knows(name)).then(|| format!("no action named `{name}`"))
    });
    let actions = actions.clone();
    functions.register("action", 1..=3, move |args| {
        let text = |i: usize| match args.get(i) {
            None => Ok(String::new()),
            Some(Value::Text(t)) => Ok(t.clone()),
            Some(other) => Err(format!("expected text, found {}", other.type_name())),
        };
        actions.dispatch(&text(0)?, &text(1)?, &text(2)?);
        Ok(Value::Null)
    });
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;

    use super::*;
    use crate::Expr;

    struct Env;

    impl crate::Scope for Env {
        fn roots(&self) -> Vec<String> {
            Vec::new()
        }

        fn get(&self, _: &[String]) -> Option<Value> {
            None
        }
    }

    #[test]
    fn markup_dispatches_through_the_registry_and_checks_names() {
        let actions = Actions::new("stack");
        let seen = Rc::new(RefCell::new(Vec::new()));
        let sink = seen.clone();
        actions.register(&["panel"], None, move |verb, extra| sink.borrow_mut().push((verb, extra.to_string())), || {});
        let mut f = Functions::standard();
        bind_actions(&mut f, &actions);
        let run = |src: &str| Expr::parse(src).unwrap().eval(&Env, &f);
        run("action(\"panel\")").unwrap();
        run("action(\"open\", \"panel\", \"x\")").unwrap();
        assert_eq!(*seen.borrow(), vec![(Verb::Toggle, String::new()), (Verb::Open, "x".to_string())]);
        assert!(run("action(1)").is_err());
        let bad = Expr::parse("action(\"panle\")").unwrap().check(&Env, &f);
        assert_eq!(bad.len(), 1);
        assert!(bad[0].message.contains("no action named `panle`"), "{}", bad[0].message);
        assert!(Expr::parse("action(\"close\", \"panel\")").unwrap().check(&Env, &f).is_empty());
    }
}
