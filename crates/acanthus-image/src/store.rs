use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::path::PathBuf;
use std::rc::{Rc, Weak};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::time::UNIX_EPOCH;

use acanthus_core::Signal;

use crate::decode::{Decoded, decode_file};
use crate::icons::IconLookup;
use crate::source::ImageSource;

const WORKERS: usize = 2;
const DEFAULT_ICON_SIZE: u32 = 48;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
/// Identifier of an image in an [`ImageStore`].
pub struct ImageId(u64);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
/// Load state of an image, like QML `Image.status`.
pub enum ImageStatus {
    #[default]
    /// No image requested.
    Null,
    /// Decoding on a worker thread.
    Loading,
    /// Decoded and available through [`ImageStore::get`].
    Ready,
    /// Decoding failed.
    Error,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct Key {
    source: ImageSource,
    target: Option<(u32, u32)>,
    mtime: Option<u64>,
}

struct Job {
    id: ImageId,
    key: Key,
}

struct Done {
    id: ImageId,
    result: Result<Decoded, String>,
}

struct Slot {
    key: Key,
    status: ImageStatus,
    decoded: Option<Arc<Decoded>>,
    refs: usize,
}

struct Inner {
    keys: RefCell<HashMap<Key, ImageId>>,
    slots: RefCell<HashMap<ImageId, Slot>>,
    next: Cell<u64>,
    jobs: Sender<Job>,
    results: Receiver<Done>,
    generation: Signal<u64>,
    icons: Arc<IconLookup>,
    workers: RefCell<Option<WorkerSeed>>,
}

struct WorkerSeed {
    job_rx: Arc<Mutex<Receiver<Job>>>,
    done_tx: Sender<Done>,
    waker: Option<Arc<dyn Fn() + Send + Sync>>,
}

#[derive(Clone)]
/// Asynchronous image cache. Equal requests share one slot, decoding runs on worker threads, results are applied by [`ImageStore::poll`] on the UI thread.
pub struct ImageStore {
    inner: Rc<Inner>,
}

/// A counted reference to an image; the slot is freed when the last handle drops.
pub struct ImageHandle {
    id: ImageId,
    store: Weak<Inner>,
}

impl ImageHandle {
    /// The identifier of the image.
    pub fn id(&self) -> ImageId {
        self.id
    }
}

impl Drop for ImageHandle {
    fn drop(&mut self) {
        let Some(inner) = self.store.upgrade() else { return };
        let mut slots = inner.slots.borrow_mut();
        let Some(slot) = slots.get_mut(&self.id) else { return };
        slot.refs -= 1;
        if slot.refs == 0 {
            let slot = slots.remove(&self.id).unwrap();
            inner.keys.borrow_mut().remove(&slot.key);
        }
    }
}

impl ImageStore {
    /// Store using the icon lookup of the environment.
    pub fn new() -> Self {
        ImageStore::with_icons(Arc::new(IconLookup::from_environment()))
    }

    /// Store using the given icon lookup.
    pub fn with_icons(icons: Arc<IconLookup>) -> Self {
        ImageStore::build(icons, None)
    }

    /// Like [`ImageStore::with_icons`], and `waker` is called from worker threads whenever a result is ready.
    pub fn with_waker(icons: Arc<IconLookup>, waker: Arc<dyn Fn() + Send + Sync>) -> Self {
        ImageStore::build(icons, Some(waker))
    }

    fn build(icons: Arc<IconLookup>, waker: Option<Arc<dyn Fn() + Send + Sync>>) -> Self {
        let (job_tx, job_rx) = mpsc::channel::<Job>();
        let (done_tx, done_rx) = mpsc::channel::<Done>();
        ImageStore {
            inner: Rc::new(Inner {
                keys: RefCell::new(HashMap::new()),
                slots: RefCell::new(HashMap::new()),
                next: Cell::new(1),
                jobs: job_tx,
                results: done_rx,
                generation: Signal::new(0),
                icons,
                workers: RefCell::new(Some(WorkerSeed { job_rx: Arc::new(Mutex::new(job_rx)), done_tx, waker })),
            }),
        }
    }

    fn start_workers(&self) {
        let Some(seed) = self.inner.workers.borrow_mut().take() else { return };
        for n in 0..WORKERS {
            let job_rx = seed.job_rx.clone();
            let done_tx = seed.done_tx.clone();
            let icons = self.inner.icons.clone();
            let waker = seed.waker.clone();
            let spawned = std::thread::Builder::new().name(format!("acanthus-image-{n}")).spawn(move || {
                loop {
                    let job = match job_rx.lock().unwrap().recv() {
                        Ok(job) => job,
                        Err(_) => return,
                    };
                    let result = run(&icons, &job.key);
                    if done_tx.send(Done { id: job.id, result }).is_err() {
                        return;
                    }
                    if let Some(wake) = &waker {
                        wake();
                    }
                }
            });
            if let Err(e) = spawned {
                tracing::warn!("cannot start image worker: {e}");
            }
        }
    }

    /// Whether both stores are the same one.
    pub fn ptr_eq(&self, other: &ImageStore) -> bool {
        Rc::ptr_eq(&self.inner, &other.inner)
    }

    /// The icon lookup of the store.
    pub fn icons(&self) -> &Arc<IconLookup> {
        &self.inner.icons
    }

    /// Signal bumped each time [`ImageStore::poll`] applies results, for effects that depend on image state.
    pub fn generation(&self) -> Signal<u64> {
        self.inner.generation.clone()
    }

    /// Requests an image, optionally rasterised at `target`. Returns `None` for an empty source. In-memory sources are ready at once.
    pub fn request(&self, source: &str, target: Option<(u32, u32)>) -> Option<ImageHandle> {
        let source = ImageSource::parse(source);
        if source == ImageSource::None {
            return None;
        }
        let mtime = match &source {
            ImageSource::File(path) => std::fs::metadata(path)
                .and_then(|m| m.modified())
                .ok()
                .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
                .map(|d| d.as_nanos() as u64),
            _ => None,
        };
        let key = Key { source, target, mtime };
        let inner = &self.inner;
        let existing = inner.keys.borrow().get(&key).copied();
        if let Some(id) = existing {
            if let Some(slot) = inner.slots.borrow_mut().get_mut(&id) {
                slot.refs += 1;
            }
            return Some(ImageHandle { id, store: Rc::downgrade(inner) });
        }
        let id = ImageId(inner.next.get());
        inner.next.set(id.0 + 1);
        inner.keys.borrow_mut().insert(key.clone(), id);
        if let ImageSource::Memory(memory) = key.source {
            let decoded = crate::memory::lookup(memory);
            let status = if decoded.is_some() { ImageStatus::Ready } else { ImageStatus::Error };
            inner.slots.borrow_mut().insert(id, Slot { key, status, decoded, refs: 1 });
            return Some(ImageHandle { id, store: Rc::downgrade(inner) });
        }
        self.start_workers();
        inner
            .slots
            .borrow_mut()
            .insert(id, Slot { key: key.clone(), status: ImageStatus::Loading, decoded: None, refs: 1 });
        if inner.jobs.send(Job { id, key }).is_err()
            && let Some(slot) = inner.slots.borrow_mut().get_mut(&id)
        {
            slot.status = ImageStatus::Error;
        }
        Some(ImageHandle { id, store: Rc::downgrade(inner) })
    }

    /// Applies finished decodes. Returns whether anything changed.
    pub fn poll(&self) -> bool {
        let mut changed = false;
        while let Ok(done) = self.inner.results.try_recv() {
            let mut slots = self.inner.slots.borrow_mut();
            let Some(slot) = slots.get_mut(&done.id) else { continue };
            match done.result {
                Ok(decoded) => {
                    slot.decoded = Some(Arc::new(decoded));
                    slot.status = ImageStatus::Ready;
                }
                Err(message) => {
                    tracing::warn!("cannot load image: {message}");
                    slot.status = ImageStatus::Error;
                }
            }
            changed = true;
        }
        if changed {
            self.inner.generation.update(|g| *g += 1);
        }
        changed
    }

    /// Load state of `id`, [`ImageStatus::Null`] if unknown.
    pub fn status(&self, id: ImageId) -> ImageStatus {
        self.inner.slots.borrow().get(&id).map_or(ImageStatus::Null, |s| s.status)
    }

    /// The decoded image once ready.
    pub fn get(&self, id: ImageId) -> Option<Arc<Decoded>> {
        self.inner.slots.borrow().get(&id).and_then(|s| s.decoded.clone())
    }

    /// Number of live slots.
    pub fn len(&self) -> usize {
        self.inner.slots.borrow().len()
    }

    /// Whether there are no live slots.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

impl Default for ImageStore {
    fn default() -> Self {
        ImageStore::new()
    }
}

fn run(icons: &IconLookup, key: &Key) -> Result<Decoded, String> {
    let path: PathBuf = match &key.source {
        ImageSource::None => return Err("empty source".to_owned()),
        ImageSource::Memory(_) => return Err("memory images resolve on request".to_owned()),
        ImageSource::File(path) => path.clone(),
        ImageSource::Icon(name) => {
            let size = key.target.map_or(DEFAULT_ICON_SIZE, |(w, h)| w.max(h));
            icons.lookup(name, size).ok_or_else(|| format!("icon {name} not found"))?
        }
    };
    decode_file(&path, key.target).map_err(|e| format!("{}: {e}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    fn store() -> ImageStore {
        ImageStore::with_icons(Arc::new(IconLookup::new(Vec::new(), Vec::new(), "hicolor")))
    }

    fn settle(store: &ImageStore) {
        let start = Instant::now();
        while start.elapsed() < Duration::from_secs(5) {
            store.poll();
            if store.inner.slots.borrow().values().all(|s| s.status != ImageStatus::Loading) {
                return;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        panic!("images did not finish loading");
    }

    fn write_png(path: &std::path::Path) {
        image::RgbaImage::from_pixel(6, 3, image::Rgba([10, 20, 30, 255])).save(path).unwrap();
    }

    #[test]
    fn loads_in_the_background_and_signals_the_generation() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a.png");
        write_png(&path);
        let store = store();
        let generation = store.generation();
        let handle = store.request(&format!("file://{}?rev=1", path.display()), None).unwrap();
        assert_eq!(store.status(handle.id()), ImageStatus::Loading);
        settle(&store);
        assert_eq!(store.status(handle.id()), ImageStatus::Ready);
        let decoded = store.get(handle.id()).unwrap();
        assert_eq!((decoded.width, decoded.height), (6, 3));
        assert!(generation.get_untracked() >= 1);
    }

    #[test]
    fn memory_images_are_ready_on_request_without_a_copy() {
        let slot = crate::MemorySlot::new();
        let image = Arc::new(Decoded { width: 1, height: 1, rgba: vec![1, 2, 3, 4], animation: None });
        let store = store();
        let handle = store.request(&slot.set_shared(image.clone()), None).unwrap();
        assert_eq!(store.status(handle.id()), ImageStatus::Ready);
        assert!(Arc::ptr_eq(&store.get(handle.id()).unwrap(), &image));
        drop(slot);
        let gone = store.request(&format!("image://mem/{:016x}", u64::MAX), None).unwrap();
        assert_eq!(store.status(gone.id()), ImageStatus::Error);
    }

    #[test]
    fn identical_requests_share_one_slot_and_release_frees_it() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a.png");
        write_png(&path);
        let store = store();
        let a = store.request(path.to_str().unwrap(), None).unwrap();
        let b = store.request(path.to_str().unwrap(), None).unwrap();
        assert_eq!(a.id(), b.id());
        let sized = store.request(path.to_str().unwrap(), Some((3, 3))).unwrap();
        assert_ne!(a.id(), sized.id());
        assert_eq!(store.len(), 2);
        drop(a);
        assert_eq!(store.len(), 2);
        drop(b);
        assert_eq!(store.len(), 1);
        drop(sized);
        assert!(store.is_empty());
    }

    #[test]
    fn missing_files_and_unknown_icons_report_errors() {
        let store = store();
        let file = store.request("/no/such/file.png", None).unwrap();
        let icon = store.request("image://icon/definitely-missing", Some((16, 16))).unwrap();
        assert!(store.request("", None).is_none());
        settle(&store);
        assert_eq!(store.status(file.id()), ImageStatus::Error);
        assert_eq!(store.status(icon.id()), ImageStatus::Error);
    }
}
