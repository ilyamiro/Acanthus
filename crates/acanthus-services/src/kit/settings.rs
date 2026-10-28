use acanthus_core::{Memo, Signal};
use serde_json::Value;

pub fn derive<T: PartialEq + 'static>(source: &Signal<Value>, map: impl Fn(&Value) -> T + 'static) -> Memo<T> {
    let source = source.clone();
    Memo::new(move || source.with(&map))
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;
    use std::rc::Rc;

    use acanthus_core::Effect;
    use serde_json::json;

    use super::*;

    #[test]
    fn the_derived_signal_only_changes_when_its_own_value_does() {
        let source = Signal::new(json!({"a": 1, "b": 1}));
        let derived = derive(&source, |value| value["a"].as_i64().unwrap_or(0));
        let runs = Rc::new(Cell::new(0));
        let counter = runs.clone();
        let watched = derived.signal.clone();
        let _watch = Effect::new(move || {
            watched.get();
            counter.set(counter.get() + 1);
        });
        assert_eq!(runs.get(), 1);
        source.set(json!({"a": 1, "b": 2}));
        assert_eq!(runs.get(), 1);
        source.set(json!({"a": 2, "b": 2}));
        assert_eq!((runs.get(), derived.signal.get_untracked()), (2, 2));
    }
}
