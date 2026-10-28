use acanthus_core::{Color, Signal};
use acanthus_scene::{Anchors, Node, Scene};

pub struct Inputs {
    pub time: Signal<String>,
    pub title: Signal<String>,
    pub artist: Signal<String>,
}

pub fn build(scene: &mut Scene, inputs: Inputs) {
    let root = scene.root();
    let card = scene
        .add(root, Node::rect().color(Color::hex(0x1e1e2e).with_alpha(0.7)).radius(20.0).anchors(Anchors::fill(0.0)));
    scene.add(
        card,
        Node::text(inputs.time)
            .color(Color::hex(0xcdd6f4))
            .font_size(36.0)
            .anchors(Anchors::default().left(20.0).top(14.0)),
    );
    scene.add(
        card,
        Node::text(inputs.title)
            .color(Color::hex(0x89b4fa))
            .font_size(14.0)
            .anchors(Anchors::default().left(20.0).bottom(36.0)),
    );
    scene.add(
        card,
        Node::text(inputs.artist)
            .color(Color::hex(0xa6adc8))
            .font_size(12.0)
            .anchors(Anchors::default().left(20.0).bottom(16.0)),
    );
}
