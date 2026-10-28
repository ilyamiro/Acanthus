use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::{Rc, Weak};

use acanthus_core::{Signal, batch};
use acanthus_io::Watch;
use calloop::LoopHandle;
use serde_json::Value;

const RTL_LANGUAGES: &[&str] = &["ar", "fa", "ur", "he", "iw", "ps", "ug", "ckb", "yi"];

struct Inner {
    dir: PathBuf,
    lang: Signal<String>,
    fallback: RefCell<Value>,
    current: RefCell<Value>,
    revision: Signal<u64>,
    ready: Signal<bool>,
    watch: RefCell<Option<Watch>>,
}

#[derive(Clone)]
pub struct I18n(Rc<Inner>);

pub fn system_language(env: impl Fn(&str) -> Option<String>) -> String {
    let raw = ["LC_ALL", "LC_MESSAGES", "LANG", "LANGUAGE"]
        .iter()
        .filter_map(|k| env(k))
        .find(|v| !v.is_empty() && v != "C" && v != "POSIX")
        .unwrap_or_else(|| "en_US".to_owned());
    let name = raw.split(['.', '@', ':']).next().unwrap_or("en_US").to_owned();
    let lang = name.split('_').next().unwrap_or("en").to_lowercase();
    let territory = name.split('_').nth(1).unwrap_or("").to_uppercase();
    match lang.as_str() {
        "zh" if matches!(territory.as_str(), "TW" | "HK" | "MO") => "zh_tw".into(),
        "uk" => "ua".into(),
        "" => "en".into(),
        _ => lang,
    }
}

pub fn is_rtl(lang: &str) -> bool {
    RTL_LANGUAGES.contains(&lang)
}

fn read_json(path: &Path) -> Option<Value> {
    let text = std::fs::read_to_string(path).ok()?;
    if text.trim().is_empty() {
        return None;
    }
    serde_json::from_str(&text).map_err(|e| tracing::warn!("{}: {e}", path.display())).ok()
}

fn resolve<'a>(root: &'a Value, key: &str) -> Option<&'a str> {
    key.split('.').try_fold(root, |cur, part| cur.as_object()?.get(part))?.as_str()
}

impl I18n {
    pub fn new(dir: &Path, lang: &str) -> I18n {
        let i18n = I18n(Rc::new(Inner {
            dir: dir.to_path_buf(),
            lang: Signal::new(lang.to_owned()),
            fallback: RefCell::new(Value::Null),
            current: RefCell::new(Value::Null),
            revision: Signal::new(0),
            ready: Signal::new(false),
            watch: RefCell::new(None),
        }));
        i18n.load_files();
        i18n
    }

    fn load_files(&self) {
        let lang = self.0.lang.get_untracked();
        let fallback = read_json(&self.0.dir.join("en.json"));
        let current = if lang == "en" { None } else { read_json(&self.0.dir.join(format!("{lang}.json"))) };
        let ready = fallback.is_some();
        *self.0.fallback.borrow_mut() = fallback.unwrap_or(Value::Null);
        *self.0.current.borrow_mut() = current.unwrap_or(Value::Null);
        batch(|| {
            self.0.ready.set(ready);
            self.0.revision.update(|r| *r += 1);
        });
    }

    pub fn language(&self) -> Signal<String> {
        self.0.lang.clone()
    }

    pub fn ready(&self) -> Signal<bool> {
        self.0.ready.clone()
    }

    pub fn revision(&self) -> Signal<u64> {
        self.0.revision.clone()
    }

    pub fn set_language(&self, lang: &str) {
        if self.0.lang.get_untracked() == lang {
            return;
        }
        batch(|| {
            self.0.lang.set(lang.to_owned());
            self.load_files();
        });
    }

    pub fn rtl(&self) -> bool {
        is_rtl(&self.0.lang.get())
    }

    pub fn translate(&self, key: &str) -> Option<String> {
        self.0.revision.get();
        let lang = self.0.lang.get();
        if !self.0.ready.get() {
            return None;
        }
        let from_current = (lang != "en").then(|| resolve(&self.0.current.borrow(), key).map(str::to_owned)).flatten();
        from_current.or_else(|| resolve(&self.0.fallback.borrow(), key).map(str::to_owned))
    }

    pub fn t(&self, key: &str) -> String {
        self.translate(key).unwrap_or_else(|| key.to_owned())
    }

    pub fn tr(&self, key: &str, fallback: &str) -> String {
        self.translate(key).unwrap_or_else(|| fallback.to_owned())
    }

    pub fn t_args(&self, key: &str, args: &[(&str, &str)]) -> String {
        let mut text = self.t(key);
        for (name, value) in args {
            text = text.replace(&format!("{{{name}}}"), value);
        }
        text
    }

    pub fn available_languages(&self) -> Vec<String> {
        let mut langs: Vec<String> = std::fs::read_dir(&self.0.dir)
            .into_iter()
            .flatten()
            .flatten()
            .filter_map(|e| e.file_name().to_str()?.strip_suffix(".json").map(str::to_owned))
            .collect();
        langs.sort();
        langs
    }

    pub fn attach<D: 'static>(&self, handle: &LoopHandle<'static, D>) -> Result<(), acanthus_io::IoError> {
        let weak: Weak<Inner> = Rc::downgrade(&self.0);
        let watch = Watch::directory(handle, &self.0.dir, move |event| {
            let Some(inner) = weak.upgrade() else { return };
            let name = event.name.to_string_lossy().into_owned();
            let lang = inner.lang.get_untracked();
            if name == "en.json" || name == format!("{lang}.json") {
                I18n(inner).load_files();
            }
        })?;
        *self.0.watch.borrow_mut() = Some(watch);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;
    use std::time::{Duration, Instant};

    use acanthus_core::Effect;
    use calloop::EventLoop;

    use super::*;

    fn dir_with(files: &[(&str, &str)]) -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        for (name, text) in files {
            std::fs::write(dir.path().join(name), text).unwrap();
        }
        dir
    }

    fn env_of(pairs: &'static [(&'static str, &'static str)]) -> impl Fn(&str) -> Option<String> {
        move |k| pairs.iter().find(|(key, _)| *key == k).map(|(_, v)| v.to_string())
    }

    #[test]
    fn nested_keys_arguments_and_fallback_chain() {
        let dir = dir_with(&[
            ("en.json", r#"{"a": {"b": "Hello {name}", "only_en": "English"}, "n": "{x} of {y}"}"#),
            ("de.json", r#"{"a": {"b": "Hallo {name}"}}"#),
        ]);
        let i18n = I18n::new(dir.path(), "de");
        assert_eq!(i18n.t_args("a.b", &[("name", "Ada")]), "Hallo Ada");
        assert_eq!(i18n.t("a.only_en"), "English");
        assert_eq!(i18n.t("missing.key"), "missing.key");
        assert_eq!(i18n.tr("missing.key", "Fallback"), "Fallback");
        assert_eq!(i18n.t_args("n", &[("x", "1"), ("y", "2")]), "1 of 2");
    }

    #[test]
    fn switching_language_reruns_dependent_effects() {
        let dir = dir_with(&[("en.json", r#"{"hi": "Hi"}"#), ("fr.json", r#"{"hi": "Salut"}"#)]);
        let i18n = I18n::new(dir.path(), "en");
        let seen = Rc::new(RefCell::new(String::new()));
        let (s, i) = (seen.clone(), i18n.clone());
        let _e = Effect::new(move || *s.borrow_mut() = i.t("hi"));
        assert_eq!(*seen.borrow(), "Hi");
        i18n.set_language("fr");
        assert_eq!(*seen.borrow(), "Salut");
        i18n.set_language("zz");
        assert_eq!(*seen.borrow(), "Hi");
    }

    #[test]
    fn rtl_follows_language() {
        let dir = dir_with(&[("en.json", "{}")]);
        let i18n = I18n::new(dir.path(), "ar");
        assert!(i18n.rtl());
        i18n.set_language("de");
        assert!(!i18n.rtl());
    }

    #[test]
    fn system_language_normalisation() {
        assert_eq!(system_language(env_of(&[("LANG", "de_DE.UTF-8")])), "de");
        assert_eq!(system_language(env_of(&[("LANG", "uk_UA.UTF-8")])), "ua");
        assert_eq!(system_language(env_of(&[("LANG", "zh_TW.UTF-8")])), "zh_tw");
        assert_eq!(system_language(env_of(&[("LANG", "zh_CN.UTF-8")])), "zh");
        assert_eq!(system_language(env_of(&[("LC_ALL", "fr_FR"), ("LANG", "de_DE")])), "fr");
        assert_eq!(system_language(env_of(&[("LANG", "C")])), "en");
        assert_eq!(system_language(env_of(&[])), "en");
    }

    #[test]
    fn every_bundled_language_loads_and_translates() {
        let paths = crate::paths::test_paths();
        let i18n = I18n::new(&paths.languages_dir(), "en");
        let langs = i18n.available_languages();
        assert!(langs.len() >= 20, "{langs:?}");
        for lang in &langs {
            i18n.set_language(lang);
            assert!(i18n.ready().get_untracked());
            assert_ne!(i18n.t("guide.tabs.general"), "guide.tabs.general", "{lang}");
        }
    }

    #[test]
    fn edited_language_file_reloads() {
        let dir = dir_with(&[("en.json", r#"{"k": "one"}"#)]);
        let mut event_loop: EventLoop<'static, ()> = EventLoop::try_new().unwrap();
        let i18n = I18n::new(dir.path(), "en");
        i18n.attach(&event_loop.handle()).unwrap();
        std::fs::write(dir.path().join("en.json"), r#"{"k": "two"}"#).unwrap();
        let hits = Cell::new(0);
        let end = Instant::now() + Duration::from_millis(200);
        while Instant::now() < end && i18n.t("k") != "two" {
            event_loop.dispatch(Duration::from_millis(10), &mut ()).unwrap();
            hits.set(hits.get() + 1);
        }
        assert_eq!(i18n.t("k"), "two");
    }
}
