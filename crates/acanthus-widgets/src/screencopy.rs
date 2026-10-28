//! A live picture of pixels that change over time, such as a screen capture: an image node whose source follows a frame signal and holds only the latest frame in memory.

use std::rc::Rc;
use std::sync::Arc;

use acanthus_anim::Keep;
use acanthus_core::Signal;
use acanthus_scene::{Decoded, FillMode, MemorySlot, Node};

/// An image node showing the latest frame of `frame` (straight RGBA8), empty while it is `None`.
pub fn screencopy_view(frame: Signal<Option<Arc<Decoded>>>, fill: FillMode) -> Node {
    let slot = Rc::new(MemorySlot::new());
    let mut keep = Keep::default();
    let source = {
        let slot = slot.clone();
        keep.derive(move || frame.with(|f| f.as_ref().map(|image| slot.set_shared(image.clone())).unwrap_or_default()))
    };
    Node::image(source).fill_mode(fill).own(keep).own(slot)
}

#[cfg(test)]
mod tests {
    use super::*;
    use acanthus_core::Vec2;
    use acanthus_scene::{Fonts, ImageStatus, Scene};

    fn settle(scene: &mut Scene) {
        let start = std::time::Instant::now();
        while !scene.images().poll() && start.elapsed() < std::time::Duration::from_secs(5) {
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
    }

    fn solid(width: u32, height: u32, value: u8) -> Arc<Decoded> {
        Arc::new(Decoded { width, height, rgba: vec![value; (width * height * 4) as usize], animation: None })
    }

    #[test]
    fn the_view_loads_each_new_frame_and_follows_its_size() {
        let frame = Signal::new(None);
        let node = screencopy_view(frame.clone(), FillMode::Stretch);
        let status = node.image_status().unwrap();
        let implicit = node.implicit_size();
        let mut scene = Scene::new(Fonts::new());
        let root = scene.root();
        scene.add(root, node);
        scene.frame();
        assert_eq!(status.get_untracked(), ImageStatus::Null);

        frame.set(Some(solid(16, 9, 10)));
        scene.frame();
        settle(&mut scene);
        scene.frame();
        assert_eq!(status.get_untracked(), ImageStatus::Ready);
        assert_eq!(implicit.get_untracked(), Vec2::new(16.0, 9.0));

        frame.set(Some(solid(32, 18, 20)));
        scene.frame();
        settle(&mut scene);
        scene.frame();
        assert_eq!(implicit.get_untracked(), Vec2::new(32.0, 18.0));
    }
}
