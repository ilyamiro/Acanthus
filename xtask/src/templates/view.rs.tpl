use std::rc::Rc;

use acanthus_scene::Node;

use super::controller::Controller;

pub fn build(controller: &Rc<Controller>) -> Node {
    Node::text(controller.label.clone())
}
