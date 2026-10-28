use acanthus_core::{Color, Signal};
use acanthus_scene::{Anchors, Node, Scene};

pub struct Inputs {
    pub time: Signal<String>,
    pub battery: Signal<String>,
}

pub fn build(scene: &mut Scene, inputs: Inputs) {
    let root = scene.root();
    let background = scene.add(root, Node::rect().color(Color::hex(0x1e1e2e)).anchors(Anchors::fill(0.0)));
    scene.add(
        background,
        Node::text(inputs.time).color(Color::hex(0xcdd6f4)).font_size(15.0).anchors(Anchors::center()),
    );
    scene.add(
        background,
        Node::text(inputs.battery)
            .color(Color::hex(0xa6e3a1))
            .font_size(13.0)
            .anchors(Anchors::default().right(12.0).v_center()),
    );
    scene.add(
        background,
        Node::text("hello-bar")
            .color(Color::hex(0x89b4fa))
            .font_size(13.0)
            .anchors(Anchors::default().left(12.0).v_center()),
    );
}
