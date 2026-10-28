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
    fn the_oldest_images_are_dropped_past_the_capacity() {
        let first = register(1, 1, &[9, 9, 9, 9]);
        for n in 0..CAPACITY as u32 {
            register(2, 1, &[n as u8, (n >> 8) as u8, 7, 7, 0, 0, 0, 0]);
        }
        assert!(lookup(parse(&first).unwrap()).is_none());
    }
}
