use acanthus_app::Popup;
use acanthus_app::popup::closing_on_escape;
use acanthus_core::Color;
use acanthus_scene::{Anchors, Node, Scene};

use crate::controller::Controller;
use crate::model::{HEIGHT, WIDTH};

pub fn build(scene: &mut Scene, controller: Controller) {
    let root = scene.root();
    let close = controller.clone();
    let card = scene.add(
        root,
        Node::rect()
            .color(Color::hex(0x1e1e2e))
            .radius(16.0)
            .width(WIDTH)
            .height(HEIGHT)
            .anchors(Anchors::center())
            .visible(&controller.open)
            .focusable(true)
            .on_key(closing_on_escape(move || close.close(), |_| false)),
    );
    scene
        .add(card, Node::text(&controller.text).color(Color::hex(0xcdd6f4)).font_size(22.0).anchors(Anchors::center()));
    scene.set_focus(Some(card));
}
