use acanthus_core::{Effect, Memo, Signal, untrack};
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::{Value, json};

use crate::config::Config;

/// A typed view of one settings path: the value is deserialised with serde, keeps the last good value when the
/// stored JSON stops matching the type, and reports what went wrong in `errors`.
pub struct Section<T: 'static> {
    /// The deserialised section; only changes when the parsed value differs.
    pub value: Signal<T>,
    /// Validation messages for the current stored JSON, empty when it parses.
    pub errors: Signal<Vec<String>>,
    path: String,
    _binding: Effect,
}

impl<T: Clone + PartialEq + 'static> Section<T> {
    /// Settings path this section reads, in dotted form.
    pub fn path(&self) -> &str {
        &self.path
    }

    /// Current value without subscribing.
    pub fn get_untracked(&self) -> T {
        self.value.get_untracked()
    }

    /// A field selector: the returned memo notifies only when the selected field changes, so a consumer of one
    /// field is not re-run by edits to its siblings. Keep the memo alive as long as the consumer.
    pub fn select<U: PartialEq + 'static>(&self, field: impl Fn(&T) -> U + 'static) -> Memo<U> {
        let value = self.value.clone();
        Memo::new(move || value.with(&field))
    }
}

fn kind(value: &Value) -> &'static str {
    match value {
        Value::Null => "null",
        Value::Bool(_) => "boolean",
        Value::Number(number) if number.is_f64() => "number",
        Value::Number(_) => "integer",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
}

fn describe(value: &Value) -> Value {
    match value {
        Value::Object(map) => {
            let fields: serde_json::Map<String, Value> =
                map.iter().map(|(name, leaf)| (name.clone(), describe(leaf))).collect();
            json!({"type": "object", "default": value, "fields": fields})
        }
        other => json!({"type": kind(other), "default": other}),
    }
}

impl<T: Serialize + Default + 'static> Section<T> {
    /// Machine-readable description of the section for a settings editor: the path, and for every field (nested
    /// objects included) its JSON type and default, taken from `T::default()` serialised with the serde names.
    pub fn schema(&self) -> Value {
        let mut schema = describe(&serde_json::to_value(T::default()).unwrap_or(Value::Null));
        schema["path"] = Value::String(self.path.clone());
        schema
    }
}

impl Config {
    /// Binds `path` to a typed [`Section`]. Missing or `null` leaves come from the defaults and then from `T::default()`
    /// through `#[serde(default)]`; a value of the wrong type keeps the previous value and fills `errors`.
    pub fn section<T>(&self, path: &str) -> Section<T>
    where
        T: DeserializeOwned + Default + Clone + PartialEq + 'static,
    {
        self.section_with(&[path], |values| values[0].clone())
    }

    /// Like [`Config::section`] for settings spread over several paths or stored in older layouts: `shape` receives
    /// the JSON at each of `paths` (in order, `null` when unset) and returns the JSON `T` is parsed from, so legacy
    /// keys and fallbacks map onto one serde type. The section reports the first path as its own.
    pub fn section_with<T>(&self, paths: &[&str], shape: impl Fn(&[Value]) -> Value + 'static) -> Section<T>
    where
        T: DeserializeOwned + Default + Clone + PartialEq + 'static,
    {
        let leaves: Vec<Signal<Value>> = paths.iter().map(|path| self.value(path)).collect();
        let value = Signal::new(T::default());
        let errors = Signal::new(Vec::new());
        let label = paths.first().copied().unwrap_or_default().to_owned();
        let (target, problems, name) = (value.clone(), errors.clone(), label.clone());
        let binding = Effect::new(move || {
            let raw: Vec<Value> = leaves.iter().map(Signal::get).collect();
            let json = shape(&raw);
            let parsed = if json.is_null() { Ok(T::default()) } else { serde_json::from_value::<T>(json) };
            untrack(|| match parsed {
                Ok(next) => {
                    if target.get_untracked() != next {
                        target.set(next);
                    }
                    if !problems.get_untracked().is_empty() {
                        problems.set(Vec::new());
                    }
                }
                Err(error) => problems.set(vec![format!("{name}: {error}")]),
            });
        });
        Section { value, errors, path: label, _binding: binding }
    }
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;
    use std::rc::Rc;

    use serde::{Deserialize, Serialize};
    use serde_json::json;

    use super::*;

    #[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
    #[serde(default, rename_all = "camelCase")]
    struct Bar {
        height: u32,
        show_clock: bool,
    }

    impl Default for Bar {
        fn default() -> Self {
            Bar { height: 40, show_clock: true }
        }
    }

    fn config(content: &str) -> (tempfile::TempDir, Config) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        std::fs::write(&path, content).unwrap();
        let config = Config::load_with_defaults(&path, json!({"bar": {"height": 40}}));
        (dir, config)
    }

    #[test]
    fn a_section_parses_with_serde_defaults() {
        let (_dir, config) = config(r#"{"bar": {"showClock": false}}"#);
        let bar = config.section::<Bar>("bar");
        assert_eq!(bar.get_untracked(), Bar { height: 40, show_clock: false });
        assert!(bar.errors.get_untracked().is_empty());
    }

    #[test]
    fn a_wrong_type_keeps_the_last_good_value_and_reports_it() {
        let (_dir, config) = config(r#"{"bar": {"height": 52}}"#);
        let bar = config.section::<Bar>("bar");
        config.set("bar.height", json!("tall"));
        assert_eq!(bar.get_untracked().height, 52);
        assert_eq!(bar.errors.get_untracked().len(), 1);
        assert!(bar.errors.get_untracked()[0].starts_with("bar:"));
        config.set("bar.height", json!(60));
        assert_eq!(bar.get_untracked().height, 60);
        assert!(bar.errors.get_untracked().is_empty());
    }

    #[test]
    fn a_selector_ignores_edits_to_sibling_fields() {
        let (_dir, config) = config("{}");
        let bar = config.section::<Bar>("bar");
        let height = bar.select(|bar| bar.height);
        let runs = Rc::new(Cell::new(0));
        let (counter, watched) = (runs.clone(), height.signal.clone());
        let _watch = Effect::new(move || {
            watched.get();
            counter.set(counter.get() + 1);
        });
        config.set("bar.showClock", json!(false));
        assert_eq!(runs.get(), 1);
        config.set("bar.height", json!(48));
        assert_eq!((runs.get(), height.signal.get_untracked()), (2, 48));
    }

    #[test]
    fn the_schema_lists_fields_with_types_and_defaults() {
        let (_dir, config) = config("{}");
        let schema = config.section::<Bar>("bar").schema();
        assert_eq!(schema["path"], "bar");
        assert_eq!(schema["fields"]["height"], json!({"type": "integer", "default": 40}));
        assert_eq!(schema["fields"]["showClock"], json!({"type": "boolean", "default": true}));
    }

    #[derive(Debug, Clone, Default, PartialEq, Deserialize)]
    #[serde(default)]
    struct Workspaces {
        #[serde(deserialize_with = "crate::lenient::text")]
        style: Option<String>,
        #[serde(deserialize_with = "crate::lenient::integer")]
        count: Option<i64>,
        #[serde(deserialize_with = "crate::lenient::truthy")]
        hide_empty: bool,
    }

    #[test]
    fn a_shaped_section_maps_legacy_keys_from_several_paths() {
        let (_dir, config) = config(r#"{"bar": {"workspacesStyle": "numbers", "hideEmptyWorkspaces": 1}}"#);
        let section = config.section_with::<Workspaces>(&["bar", "general.workspaceCount"], |raw| {
            let (bar, fallback) = (&raw[0], &raw[1]);
            let nested = &bar["workspaces"];
            let style = bar.get("workspacesStyle").filter(|v| crate::lenient::is_truthy(v)).unwrap_or(&nested["style"]);
            let count = bar.get("workspaceCount").unwrap_or(fallback);
            json!({"style": style, "count": count, "hide_empty": bar["hideEmptyWorkspaces"]})
        });
        assert_eq!(section.path(), "bar");
        assert_eq!(
            section.get_untracked(),
            Workspaces { style: Some("numbers".into()), count: None, hide_empty: true }
        );
        config.set("general.workspaceCount", json!("6"));
        assert_eq!(section.get_untracked().count, Some(6));
        config.set("bar.workspacesStyle", json!(""));
        config.set("bar.workspaces", json!({"style": "pacman"}));
        assert_eq!(section.get_untracked().style.as_deref(), Some("pacman"));
        assert!(section.errors.get_untracked().is_empty());
    }
}
