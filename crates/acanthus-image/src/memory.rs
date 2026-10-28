use std::collections::HashMap;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::sync::{Arc, Mutex, OnceLock};

use crate::decode::Decoded;

const PREFIX: &str = "image://mem/";
const CAPACITY: usize = 128;

#[derive(Default)]
struct Registry {
    images: HashMap<u64, Arc<Decoded>>,
    order: Vec<u64>,
}

fn registry() -> &'static Mutex<Registry> {
    static REGISTRY: OnceLock<Mutex<Registry>> = OnceLock::new();
    REGISTRY.get_or_init(Mutex::default)
}

/// Registers RGBA pixels and returns an in-memory image source string for [`crate::ImageStore::request`]. Identical pixels share one entry.
pub fn register(width: u32, height: u32, rgba: &[u8]) -> String {
    let mut hasher = DefaultHasher::new();
    (width, height, rgba).hash(&mut hasher);
    let id = hasher.finish();
    let mut registry = registry().lock().unwrap();
    registry.order.retain(|known| *known != id);
    registry
        .images
        .entry(id)
        .or_insert_with(|| Arc::new(Decoded { width, height, rgba: rgba.to_vec(), animation: None }));
    registry.order.push(id);
    if registry.order.len() > CAPACITY {
        let oldest = registry.order.remove(0);
        registry.images.remove(&oldest);
    }
    format!("{PREFIX}{id:016x}")
}

const SLOT_BIT: u64 = 1 << 63;

/// One replaceable in-memory image for a live source such as a screen capture: each [`MemorySlot::set`] returns a fresh source string and frees the previous frame, so memory stays at one frame regardless of the frame rate.
#[derive(Debug)]
pub struct MemorySlot {
    base: u64,
    version: std::cell::Cell<u64>,
    current: std::cell::Cell<Option<u64>>,
}

impl MemorySlot {
    /// A slot holding no image yet.
    pub fn new() -> MemorySlot {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
        let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        MemorySlot { base: SLOT_BIT | (n << 24), version: std::cell::Cell::new(0), current: std::cell::Cell::new(None) }
    }

    /// Replaces the image and returns the source string for [`crate::ImageStore::request`].
    pub fn set(&self, width: u32, height: u32, rgba: Vec<u8>) -> String {
        self.set_shared(Arc::new(Decoded { width, height, rgba, animation: None }))
    }

    /// Like [`MemorySlot::set`] for an image that is already shared, without copying its pixels.
    pub fn set_shared(&self, image: Arc<Decoded>) -> String {
        let version = self.version.get() + 1;
        self.version.set(version);
        let id = self.base | (version & 0xff_ffff);
        let mut registry = registry().lock().unwrap();
        registry.images.insert(id, image);
        if let Some(previous) = self.current.replace(Some(id)) {
            registry.images.remove(&previous);
        }
        format!("{PREFIX}{id:016x}")
    }
}

impl Default for MemorySlot {
    fn default() -> Self {
        MemorySlot::new()
    }
}

impl Drop for MemorySlot {
    fn drop(&mut self) {
        if let Some(id) = self.current.take() {
            registry().lock().unwrap().images.remove(&id);
        }
    }
}

pub(crate) fn parse(source: &str) -> Option<u64> {
    u64::from_str_radix(source.strip_prefix(PREFIX)?, 16).ok()
}

pub(crate) fn lookup(id: u64) -> Option<Arc<Decoded>> {
    registry().lock().unwrap().images.get(&id).cloned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_same_pixels_give_the_same_source_and_can_be_looked_up() {
        let a = register(1, 1, &[1, 2, 3, 4]);
        assert_eq!(a, register(1, 1, &[1, 2, 3, 4]));
        assert_ne!(a, register(1, 1, &[4, 3, 2, 1]));
        let found = lookup(parse(&a).unwrap()).unwrap();
        assert_eq!((found.width, found.height, found.rgba.clone()), (1, 1, vec![1, 2, 3, 4]));
    }

    #[test]
    fn a_slot_keeps_only_its_latest_image_and_frees_it_on_drop() {
        let slot = MemorySlot::new();
        let first = slot.set(1, 1, vec![1, 1, 1, 255]);
        let second = slot.set(2, 1, vec![2; 8]);
        assert_ne!(first, second);
        assert!(lookup(parse(&first).unwrap()).is_none());
        assert_eq!(lookup(parse(&second).unwrap()).unwrap().width, 2);
        drop(slot);
        assert!(lookup(parse(&second).unwrap()).is_none());
    }

    #[test]
    fn slots_do_not_share_ids() {
        let (a, b) = (MemorySlot::new(), MemorySlot::new());
        assert_ne!(a.set(1, 1, vec![0; 4]), b.set(1, 1, vec![0; 4]));
    }

    #[test]
    fn the_oldest_images_are_dropped_past_the_capacity() {
        let first = register(1, 1, &[9, 9, 9, 9]);
        for n in 0..CAPACITY as u32 {
            register(2, 1, &[n as u8, (n >> 8) as u8, 7, 7, 0, 0, 0, 0]);
        }
        assert!(lookup(parse(&first).unwrap()).is_none());
    }
}
