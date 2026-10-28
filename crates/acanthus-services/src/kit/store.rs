use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::{Duration, SystemTime};

use serde_json::Value;

use super::fsx;

/// Data together with the key it was fetched for and the Unix time of the fetch.
#[derive(Debug, Clone, PartialEq)]
pub struct Stored<K, D> {
    /// The key.
    pub key: K,
    /// The data.
    pub data: D,
    /// Unix time in seconds.
    pub fetched_at: i64,
}

/// Converts stored data to and from JSON.
pub trait Codec<K, D> {
    /// The JSON form of `stored`.
    fn encode(&self, stored: &Stored<K, D>) -> Value;
    /// The stored data in `value`, or `None` when it does not match.
    fn decode(&self, value: &Value) -> Option<Stored<K, D>>;
}

/// Where a feed keeps its last result.
pub trait Store<K, D> {
    /// The stored data, if any.
    fn load(&self) -> Option<Stored<K, D>>;
    /// Replaces the stored data.
    fn save(&self, stored: &Stored<K, D>);
    /// Time since the data was stored.
    fn age(&self) -> Option<Duration>;
    /// Marks the data as freshly fetched without changing it.
    fn touch(&self);
}

fn file_age(path: &Path) -> Option<Duration> {
    let modified = std::fs::metadata(path).ok()?.modified().ok()?;
    Some(SystemTime::now().duration_since(modified).unwrap_or_default())
}

fn read_json(path: &Path) -> Option<Value> {
    serde_json::from_str(std::fs::read_to_string(path).ok()?.trim()).ok()
}

/// A [`Store`] backed by one JSON file.
pub struct DiskStore<K, D> {
    service: &'static str,
    path: PathBuf,
    codec: Rc<dyn Codec<K, D>>,
}

impl<K, D> DiskStore<K, D> {
    /// A store for the file `path`; failures are logged under `service`.
    pub fn new(service: &'static str, path: PathBuf, codec: Rc<dyn Codec<K, D>>) -> DiskStore<K, D> {
        DiskStore { service, path, codec }
    }

    /// The file path.
    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl<K, D> Store<K, D> for DiskStore<K, D> {
    fn load(&self) -> Option<Stored<K, D>> {
        self.codec.decode(&read_json(&self.path)?)
    }

    fn save(&self, stored: &Stored<K, D>) {
        fsx::write_atomic(self.service, &self.path, self.codec.encode(stored).to_string().as_bytes());
    }

    fn age(&self) -> Option<Duration> {
        file_age(&self.path)
    }

    fn touch(&self) {
        fsx::touch(self.service, &self.path);
    }
}

/// A set of JSON files in one directory, one per key.
pub struct KeyedDiskStore<K, D> {
    service: &'static str,
    dir: PathBuf,
    file_name: Rc<dyn Fn(&K) -> String>,
    codec: Rc<dyn Codec<K, D>>,
}

impl<K, D> KeyedDiskStore<K, D> {
    /// A store in `dir` whose files are named by `file_name`.
    pub fn new(
        service: &'static str,
        dir: PathBuf,
        file_name: Rc<dyn Fn(&K) -> String>,
        codec: Rc<dyn Codec<K, D>>,
    ) -> KeyedDiskStore<K, D> {
        KeyedDiskStore { service, dir, file_name, codec }
    }

    /// The file that holds `key`.
    pub fn path_for(&self, key: &K) -> PathBuf {
        self.dir.join((self.file_name)(key))
    }

    /// The stored data for `key`.
    pub fn load_key(&self, key: &K) -> Option<Stored<K, D>> {
        self.codec.decode(&read_json(&self.path_for(key))?)
    }

    /// Writes the data under its own key.
    pub fn save(&self, stored: &Stored<K, D>) {
        let path = self.path_for(&stored.key);
        fsx::write_atomic(self.service, &path, self.codec.encode(stored).to_string().as_bytes());
    }

    /// Time since the data for `key` was stored.
    pub fn age_of(&self, key: &K) -> Option<Duration> {
        file_age(&self.path_for(key))
    }
}

type ReadSetting = Rc<dyn Fn() -> Value>;
type WriteSetting = Rc<dyn Fn(Value)>;

/// A [`Store`] backed by a settings value.
pub struct ConfigStore<K, D> {
    read: ReadSetting,
    write: WriteSetting,
    codec: Rc<dyn Codec<K, D>>,
    clock: Rc<dyn Fn() -> i64>,
}

impl<K, D> ConfigStore<K, D> {
    /// A store reading and writing the setting through `read` and `write`, with ages computed from `clock`.
    pub fn new(
        read: ReadSetting,
        write: WriteSetting,
        codec: Rc<dyn Codec<K, D>>,
        clock: Rc<dyn Fn() -> i64>,
    ) -> ConfigStore<K, D> {
        ConfigStore { read, write, codec, clock }
    }
}

impl<K, D> Store<K, D> for ConfigStore<K, D> {
    fn load(&self) -> Option<Stored<K, D>> {
        self.codec.decode(&(self.read)())
    }

    fn save(&self, stored: &Stored<K, D>) {
        (self.write)(self.codec.encode(stored));
    }

    fn age(&self) -> Option<Duration> {
        let stored = self.load()?;
        Some(Duration::from_secs(((self.clock)() - stored.fetched_at).max(0) as u64))
    }

    fn touch(&self) {
        if let Some(mut stored) = self.load() {
            stored.fetched_at = (self.clock)();
            self.save(&stored);
        }
    }
}

/// An in-memory [`Store`] that counts its loads and saves; for tests.
pub struct MemoryStore<K, D> {
    slot: RefCell<Option<Stored<K, D>>>,
    age: RefCell<Option<Duration>>,
    saves: RefCell<usize>,
    loads: RefCell<usize>,
}

impl<K, D> Default for MemoryStore<K, D> {
    fn default() -> Self {
        MemoryStore {
            slot: RefCell::new(None),
            age: RefCell::new(None),
            saves: RefCell::new(0),
            loads: RefCell::new(0),
        }
    }
}

impl<K: Clone, D: Clone> MemoryStore<K, D> {
    /// A store holding `stored`, reported as `age` old.
    pub fn with(stored: Stored<K, D>, age: Duration) -> MemoryStore<K, D> {
        let store = MemoryStore::default();
        *store.slot.borrow_mut() = Some(stored);
        *store.age.borrow_mut() = Some(age);
        store
    }

    /// Sets the reported age.
    pub fn set_age(&self, age: Option<Duration>) {
        *self.age.borrow_mut() = age;
    }

    /// How many times it was saved.
    pub fn saves(&self) -> usize {
        *self.saves.borrow()
    }

    /// How many times it was loaded.
    pub fn loads(&self) -> usize {
        *self.loads.borrow()
    }

    /// The data currently held.
    pub fn current(&self) -> Option<Stored<K, D>> {
        self.slot.borrow().clone()
    }
}

impl<K: Clone, D: Clone> Store<K, D> for MemoryStore<K, D> {
    fn load(&self) -> Option<Stored<K, D>> {
        *self.loads.borrow_mut() += 1;
        self.slot.borrow().clone()
    }

    fn save(&self, stored: &Stored<K, D>) {
        *self.saves.borrow_mut() += 1;
        *self.slot.borrow_mut() = Some(stored.clone());
        *self.age.borrow_mut() = Some(Duration::ZERO);
    }

    fn age(&self) -> Option<Duration> {
        *self.age.borrow()
    }

    fn touch(&self) {
        if self.slot.borrow().is_some() {
            *self.age.borrow_mut() = Some(Duration::ZERO);
        }
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    struct Plain;

    impl Codec<String, u32> for Plain {
        fn encode(&self, stored: &Stored<String, u32>) -> Value {
            json!({"key": stored.key, "data": stored.data, "at": stored.fetched_at})
        }

        fn decode(&self, value: &Value) -> Option<Stored<String, u32>> {
            Some(Stored {
                key: value.get("key")?.as_str()?.to_owned(),
                data: u32::try_from(value.get("data")?.as_u64()?).ok()?,
                fetched_at: value.get("at")?.as_i64()?,
            })
        }
    }

    fn stored(key: &str, data: u32) -> Stored<String, u32> {
        Stored { key: key.to_owned(), data, fetched_at: 7 }
    }

    #[test]
    fn a_disk_store_round_trips_through_its_codec_and_reports_its_age() {
        let dir = tempfile::tempdir().unwrap();
        let store = DiskStore::new("test", dir.path().join("x").join("a.json"), Rc::new(Plain));
        assert!(store.load().is_none());
        assert!(store.age().is_none());
        store.save(&stored("k", 3));
        assert_eq!(store.load(), Some(stored("k", 3)));
        assert!(store.age().unwrap() < Duration::from_secs(5));
        store.touch();
        std::fs::write(store.path(), "not json").unwrap();
        assert!(store.load().is_none());
    }

    #[test]
    fn a_keyed_store_keeps_one_file_per_key() {
        let dir = tempfile::tempdir().unwrap();
        let store = KeyedDiskStore::new(
            "test",
            dir.path().to_path_buf(),
            Rc::new(|key: &String| format!("{key}.json")),
            Rc::new(Plain),
        );
        store.save(&stored("a", 1));
        store.save(&stored("b", 2));
        assert_eq!(store.load_key(&"a".to_owned()), Some(stored("a", 1)));
        assert_eq!(store.load_key(&"b".to_owned()), Some(stored("b", 2)));
        assert!(store.load_key(&"c".to_owned()).is_none());
        assert!(store.age_of(&"a".to_owned()).is_some());
        assert!(dir.path().join("a.json").exists());
    }

    #[test]
    fn a_config_store_reads_and_writes_through_its_closures() {
        let cell = Rc::new(RefCell::new(Value::Null));
        let (read, write) = (cell.clone(), cell.clone());
        let now = Rc::new(std::cell::Cell::new(100));
        let clock = now.clone();
        let store = ConfigStore::new(
            Rc::new(move || read.borrow().clone()),
            Rc::new(move |value| *write.borrow_mut() = value),
            Rc::new(Plain),
            Rc::new(move || clock.get()),
        );
        assert!(store.load().is_none());
        store.save(&Stored { key: "k".to_owned(), data: 1, fetched_at: 7 });
        now.set(130);
        assert_eq!(store.age(), Some(Duration::from_secs(123)));
        store.touch();
        assert_eq!(store.load().unwrap().fetched_at, 130);
        assert_eq!(store.age(), Some(Duration::ZERO));
    }

    #[test]
    fn a_memory_store_counts_what_it_is_asked() {
        let store: MemoryStore<String, u32> = MemoryStore::default();
        assert!(store.load().is_none());
        store.save(&stored("k", 1));
        assert_eq!(store.saves(), 1);
        assert_eq!(store.loads(), 1);
        assert_eq!(store.age(), Some(Duration::ZERO));
        store.set_age(Some(Duration::from_secs(9)));
        assert_eq!(store.age(), Some(Duration::from_secs(9)));
        store.touch();
        assert_eq!(store.current().unwrap().data, 1);
    }
}
