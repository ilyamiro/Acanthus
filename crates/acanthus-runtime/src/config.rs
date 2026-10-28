use std::any::{Any, TypeId};
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::rc::{Rc, Weak};
use std::time::Duration;

use acanthus_core::{Effect, Signal, batch};
use acanthus_io::{TimerHandle, Watch, atomic_write, timer};
use calloop::LoopHandle;
use serde::de::DeserializeOwned;
use serde_json::Value;

use crate::json::{get_path, remove_path, set_path};

pub const WRITE_DEBOUNCE: Duration = Duration::from_millis(150);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfigError {
    pub message: String,
    pub line: usize,
    pub column: usize,
}

impl std::fmt::Display for ConfigError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} (line {}, column {})", self.message, self.line, self.column)
    }
}

type Scheduler = Rc<dyn Fn(Duration, Box<dyn FnOnce()>) -> Option<TimerHandle>>;

struct Inner {
    path: PathBuf,
    defaults: Value,
    raw: RefCell<Value>,
    leaves: RefCell<HashMap<String, Signal<Value>>>,
    typed: RefCell<HashMap<(String, TypeId), Box<dyn Any>>>,
    revision: Signal<u64>,
    error: Signal<Option<ConfigError>>,
    last_written: RefCell<String>,
    dirty: Cell<bool>,
    scheduler: RefCell<Option<Scheduler>>,
    write_timer: RefCell<Option<TimerHandle>>,
    watch: RefCell<Option<Watch>>,
}

#[derive(Clone)]
pub struct Config(Rc<Inner>);

fn merge_defaults(defaults: &Value, user: &Value) -> Value {
    match (defaults, user) {
        (Value::Object(d), Value::Object(u)) => {
            let mut out = d.clone();
            for (k, v) in u {
                let merged = match d.get(k) {
                    Some(base) => merge_defaults(base, v),
                    None => v.clone(),
                };
                out.insert(k.clone(), merged);
            }
            Value::Object(out)
        }
        (_, Value::Null) => defaults.clone(),
        _ => user.clone(),
    }
}

impl Config {
    pub fn load(path: &Path) -> Config {
        Config::load_with_defaults(path, Value::Null)
    }

    #[cfg(test)]
    pub(crate) fn load_demo(path: &Path) -> Config {
        Config::load_with_defaults(path, serde_json::from_str(include_str!("fixtures/demo_settings.json")).unwrap())
    }

    pub fn load_with_defaults(path: &Path, defaults: Value) -> Config {
        let config = Config(Rc::new(Inner {
            path: path.to_path_buf(),
            defaults,
            raw: RefCell::new(Value::Object(Default::default())),
            leaves: RefCell::default(),
            typed: RefCell::default(),
            revision: Signal::new(0),
            error: Signal::new(None),
            last_written: RefCell::default(),
            dirty: Cell::new(false),
            scheduler: RefCell::new(None),
            write_timer: RefCell::new(None),
            watch: RefCell::new(None),
        }));
        config.reload();
        config
    }

    pub fn path(&self) -> &Path {
        &self.0.path
    }

    pub fn revision(&self) -> Signal<u64> {
        self.0.revision.clone()
    }

    pub fn error(&self) -> Signal<Option<ConfigError>> {
        self.0.error.clone()
    }

    pub fn raw(&self) -> Value {
        self.0.raw.borrow().clone()
    }

    pub fn dump(&self) -> String {
        serde_json::to_string_pretty(&*self.0.raw.borrow()).unwrap_or_default()
    }

    pub fn current(&self, path: &str) -> Value {
        let user = get_path(&self.0.raw.borrow(), path).cloned().unwrap_or(Value::Null);
        match get_path(&self.0.defaults, path) {
            Some(base) => merge_defaults(base, &user),
            None => user,
        }
    }

    pub fn user_value(&self, path: &str) -> Option<Value> {
        get_path(&self.0.raw.borrow(), path).filter(|v| !v.is_null()).cloned()
    }

    pub fn value(&self, path: &str) -> Signal<Value> {
        if let Some(existing) = self.0.leaves.borrow().get(path) {
            return existing.clone();
        }
        let signal = Signal::new(self.current(path));
        self.0.leaves.borrow_mut().insert(path.to_owned(), signal.clone());
        signal
    }

    pub fn get<T>(&self, path: &str, default: T) -> Signal<T>
    where
        T: DeserializeOwned + PartialEq + Clone + 'static,
    {
        let key = (path.to_owned(), TypeId::of::<T>());
        if let Some(entry) = self.0.typed.borrow().get(&key)
            && let Some((signal, _)) = entry.downcast_ref::<(Signal<T>, Effect)>()
        {
            return signal.clone();
        }
        let leaf = self.value(path);
        let signal = Signal::new(default.clone());
        let target = signal.clone();
        let effect = Effect::new(move || {
            let next = leaf.with(|v| serde_json::from_value::<T>(v.clone()).ok()).unwrap_or_else(|| default.clone());
            acanthus_core::untrack(|| target.set(next));
        });
        self.0.typed.borrow_mut().insert(key, Box::new((signal.clone(), effect)));
        signal
    }

    pub fn get_now<T: DeserializeOwned>(&self, path: &str, default: T) -> T {
        serde_json::from_value(self.current(path)).unwrap_or(default)
    }

    pub fn set(&self, path: &str, value: Value) {
        self.set_many(vec![(path.to_owned(), value)]);
    }

    pub fn set_many(&self, updates: Vec<(String, Value)>) {
        let mut next = self.raw();
        for (path, value) in updates {
            set_path(&mut next, &path, value);
        }
        self.replace(next, true);
    }

    pub fn unset(&self, path: &str) {
        let mut next = self.raw();
        if remove_path(&mut next, path) {
            self.replace(next, true);
        }
    }

    fn replace(&self, next: Value, persist: bool) -> bool {
        if *self.0.raw.borrow() == next {
            return false;
        }
        *self.0.raw.borrow_mut() = next;
        self.publish();
        if persist {
            self.0.dirty.set(true);
            self.schedule_write();
        }
        true
    }

    fn publish(&self) {
        let leaves: Vec<(String, Signal<Value>)> =
            self.0.leaves.borrow().iter().map(|(k, v)| (k.clone(), v.clone())).collect();
        batch(|| {
            for (path, signal) in leaves {
                signal.set(self.current(&path));
            }
            self.0.revision.update(|r| *r += 1);
        });
    }

    pub fn reload(&self) {
        let text = match std::fs::read(&self.0.path) {
            Ok(bytes) => String::from_utf8_lossy(&bytes).into_owned(),
            Err(_) => return,
        };
        let trimmed = text.trim();
        if trimmed.is_empty() || trimmed == self.0.last_written.borrow().trim() {
            return;
        }
        match serde_json::from_str::<Value>(trimmed) {
            Ok(value) if value.is_object() => {
                self.0.error.set(None);
                if self.0.dirty.get() {
                    return;
                }
                self.replace(value, false);
            }
            Ok(_) => self.fail("settings root must be a JSON object".into(), 1, 1),
            Err(e) => self.fail(e.to_string(), e.line(), e.column()),
        }
    }

    fn fail(&self, message: String, line: usize, column: usize) {
        let error = ConfigError { message, line, column };
        tracing::error!("settings.json ignored, keeping last good config: {error}");
        self.0.error.set(Some(error));
    }

    pub fn flush(&self) {
        *self.0.write_timer.borrow_mut() = None;
        if !self.0.dirty.replace(false) {
            return;
        }
        let mut text = self.dump();
        text.push('\n');
        *self.0.last_written.borrow_mut() = text.clone();
        if let Err(e) = atomic_write(&self.0.path, text.as_bytes()) {
            tracing::error!("cannot write {}: {e}", self.0.path.display());
            self.0.dirty.set(true);
        }
    }

    fn schedule_write(&self) {
        if self.0.write_timer.borrow().is_some() {
            return;
        }
        let Some(scheduler) = self.0.scheduler.borrow().clone() else { return };
        let weak: Weak<Inner> = Rc::downgrade(&self.0);
        let timer = scheduler(
            WRITE_DEBOUNCE,
            Box::new(move || {
                if let Some(inner) = weak.upgrade() {
                    Config(inner).flush();
                }
            }),
        );
        *self.0.write_timer.borrow_mut() = timer;
    }

    pub fn attach<D: 'static>(&self, handle: &LoopHandle<'static, D>) -> Result<(), acanthus_io::IoError> {
        if let Some(dir) = self.0.path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let timer_handle = handle.clone();
        *self.0.scheduler.borrow_mut() = Some(Rc::new(move |delay, f| timer::after(&timer_handle, delay, f).ok()));
        let weak = Rc::downgrade(&self.0);
        let watch = Watch::file(handle, &self.0.path, move |_| {
            if let Some(inner) = weak.upgrade() {
                Config(inner).reload();
            }
        })?;
        *self.0.watch.borrow_mut() = Some(watch);
        if self.0.dirty.get() {
            self.schedule_write();
        }
        Ok(())
    }
}

impl Drop for Inner {
    fn drop(&mut self) {
        if self.dirty.get() {
            let mut text = serde_json::to_string_pretty(&*self.raw.borrow()).unwrap_or_default();
            text.push('\n');
            let _ = atomic_write(&self.path, text.as_bytes());
        }
    }
}

#[cfg(test)]
mod tests {
    use std::time::Instant;

    use calloop::EventLoop;
    use serde_json::json;

    use super::*;

    fn pump(event_loop: &mut EventLoop<'static, ()>, ms: u64) {
        let end = Instant::now() + Duration::from_millis(ms);
        while Instant::now() < end {
            event_loop.dispatch(Duration::from_millis(5), &mut ()).unwrap();
        }
    }

    fn counter(signal: &Signal<Value>) -> (Rc<Cell<u32>>, Effect) {
        let runs = Rc::new(Cell::new(0));
        let (r, s) = (runs.clone(), signal.clone());
        let effect = Effect::new(move || {
            s.get();
            r.set(r.get() + 1);
        });
        (runs, effect)
    }

    fn fixture(content: &str) -> (tempfile::TempDir, PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        std::fs::write(&path, content).unwrap();
        (dir, path)
    }

    #[test]
    fn user_values_override_defaults_and_missing_fall_back() {
        let (_d, path) = fixture(r#"{"bar": {"height": 55}}"#);
        let config = Config::load_demo(&path);
        assert_eq!(config.get_now("bar.height", 0), 55);
        assert_eq!(config.get_now("bar.position", String::new()), "top");
        assert_eq!(config.get_now("bar.nonexistent", 7), 7);
        assert_eq!(config.get::<i64>("bar.height", 0).get_untracked(), 55);
        assert_eq!(config.get::<String>("general.language", "xx".into()).get_untracked(), "en");
    }

    #[test]
    fn type_mismatch_uses_the_default() {
        let (_d, path) = fixture(r#"{"bar": {"height": "tall"}}"#);
        let config = Config::load_demo(&path);
        assert_eq!(config.get::<i64>("bar.height", 40).get_untracked(), 40);
    }

    #[test]
    fn hot_reload_touches_only_changed_leaves() {
        let (_d, path) = fixture(r#"{"bar": {"height": 40, "position": "top"}, "dock": {"enabled": true}}"#);
        let mut event_loop: EventLoop<'static, ()> = EventLoop::try_new().unwrap();
        let config = Config::load_demo(&path);
        config.attach(&event_loop.handle()).unwrap();
        let height = config.value("bar.height");
        let position = config.value("bar.position");
        let dock = config.value("dock.enabled");
        let bar = config.value("bar");
        let (h_runs, _h) = counter(&height);
        let (p_runs, _p) = counter(&position);
        let (d_runs, _d2) = counter(&dock);
        let (b_runs, _b) = counter(&bar);

        std::fs::write(&path, r#"{"bar": {"height": 60, "position": "top"}, "dock": {"enabled": true}}"#).unwrap();
        pump(&mut event_loop, 100);

        assert_eq!(height.get_untracked(), json!(60));
        assert_eq!((h_runs.get(), p_runs.get(), d_runs.get(), b_runs.get()), (2, 1, 1, 2));
    }

    #[test]
    fn invalid_json_keeps_last_good_and_reports_position() {
        let (_d, path) = fixture(r#"{"bar": {"height": 41}}"#);
        let mut event_loop: EventLoop<'static, ()> = EventLoop::try_new().unwrap();
        let config = Config::load_demo(&path);
        config.attach(&event_loop.handle()).unwrap();
        let height = config.get::<i64>("bar.height", 0);

        std::fs::write(&path, "{\n  \"bar\": {\"height\": 99,\n").unwrap();
        pump(&mut event_loop, 100);
        assert_eq!(height.get_untracked(), 41);
        let error = config.error().get_untracked().expect("error reported");
        assert!(error.line >= 2, "{error}");

        std::fs::write(&path, r#"{"bar": {"height": 77}}"#).unwrap();
        pump(&mut event_loop, 100);
        assert_eq!(height.get_untracked(), 77);
        assert!(config.error().get_untracked().is_none());
    }

    #[test]
    fn set_is_debounced_atomic_preserves_unknown_keys_and_ignores_its_echo() {
        let (_d, path) = fixture(r#"{"future": {"flag": [1, 2]}, "bar": {"height": 40}}"#);
        let mut event_loop: EventLoop<'static, ()> = EventLoop::try_new().unwrap();
        let config = Config::load_demo(&path);
        config.attach(&event_loop.handle()).unwrap();
        let revision = config.revision();
        let before = revision.get_untracked();

        config.set("bar.height", json!(50));
        config.set("bar.height", json!(51));
        assert_eq!(revision.get_untracked(), before + 2);
        assert!(std::fs::read_to_string(&path).unwrap().contains("40"));

        pump(&mut event_loop, 350);
        let written: Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(written["bar"]["height"], 51);
        assert_eq!(written["future"]["flag"], json!([1, 2]));
        assert_eq!(revision.get_untracked(), before + 2);
        assert!(config.error().get_untracked().is_none());
    }

    #[test]
    fn identical_set_is_a_no_op() {
        let (_d, path) = fixture(r#"{"bar": {"height": 40}}"#);
        let config = Config::load_demo(&path);
        let revision = config.revision();
        let before = revision.get_untracked();
        config.set("bar.height", json!(40));
        assert_eq!(revision.get_untracked(), before);
    }

    #[test]
    fn missing_file_starts_from_defaults_and_creates_on_first_write() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nested/settings.json");
        let config = Config::load_demo(&path);
        assert_eq!(config.get_now("bar.position", String::new()), "top");
        config.set("bar.position", json!("left"));
        config.flush();
        let written: Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(written, json!({"bar": {"position": "left"}}));
    }

    #[test]
    fn dirty_config_is_written_when_dropped() {
        let (_d, path) = fixture("{}");
        {
            let config = Config::load_demo(&path);
            config.set("general.language", json!("de"));
        }
        let written: Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(written["general"]["language"], "de");
    }

    #[test]
    fn unset_reverts_to_default() {
        let (_d, path) = fixture(r#"{"bar": {"height": 90}}"#);
        let config = Config::load_demo(&path);
        let height = config.get::<i64>("bar.height", 0);
        config.unset("bar.height");
        assert_eq!(height.get_untracked(), 40);
    }
}
