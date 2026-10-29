//! Profiling overlay: a label above everything else showing the scene's [`FrameStats`] (layout
//! and display-list time, primitive count, frame rate), refreshed at most twice a second.

use acanthus_core::{Color, Scope};
use acanthus_scene::{FrameStats, Layout, Node, NodeId, Scene};

/// Adds the overlay under `parent` at its top left corner and returns the backdrop node. `label`
/// turns the statistics into the text shown, so the caller formats and translates it.
pub fn overlay(scene: &mut Scene, parent: NodeId, label: impl Fn(&FrameStats) -> String + 'static) -> NodeId {
    let stats = scene.stats();
    let mut scope = Scope::default();
    let text = scope.derive(move || label(&stats.get()));
    let backdrop = Node::rect()
        .color(Color::rgba(0.0, 0.0, 0.0, 0.7))
        .layout(Layout::stack().padding(6.0, 4.0))
        .z(f32::MAX)
        .own(scope);
    let backdrop = scene.add(parent, backdrop);
    scene.add(backdrop, Node::text(text).font_size(12.0).color(Color::WHITE));
    backdrop
}

#[cfg(test)]
mod tests {
    use super::*;
    use acanthus_core::Vec2;
    use acanthus_scene::{Fonts, Kind};

    #[test]
    fn the_overlay_shows_the_published_statistics() {
        let mut scene = Scene::new(Fonts::new());
        scene.set_viewport(Vec2::new(200.0, 100.0), 1.0);
        let root = scene.root();
        scene.add(root, Node::rect().size(10.0, 10.0));
        let backdrop = overlay(&mut scene, root, |s| format!("{} primitives", s.primitives));
        scene.frame();
        assert!(scene.needs_frame());
        scene.frame();
        let label = scene.get(backdrop).unwrap().children()[0];
        let Kind::Text(style) = &scene.get(label).unwrap().kind else { panic!("not a text node") };
        assert_eq!(style.text.get_untracked(), format!("{} primitives", scene.stats().get_untracked().primitives));
        assert!(scene.stats().get_untracked().primitives >= 2);
    }
}
