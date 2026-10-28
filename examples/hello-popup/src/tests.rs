use std::rc::Rc;

use acanthus_app::Actions;
use acanthus_core::Vec2;
use acanthus_scene::{Fonts, Key, KeyEvent, Scene};

use crate::controller::Controller;
use crate::model::{STACK, message};
use crate::view::build;

fn rig() -> (Rc<Actions>, Controller, Scene) {
    let controller = Controller::new();
    let actions = Actions::new(STACK);
    actions.add(Rc::new(controller.clone()));
    let mut scene = Scene::new(Fonts::new());
    scene.set_viewport(Vec2::new(800.0, 600.0), 1.0);
    build(&mut scene, controller.clone());
    (actions, controller, scene)
}

#[test]
fn message_names_the_argument() {
    assert_eq!(message(""), "Hello!");
    assert_eq!(message("  Ada "), "Hello, Ada!");
}

#[test]
fn ipc_commands_open_toggle_and_close() {
    let (actions, controller, _scene) = rig();
    assert!(actions.dispatch("open", "greeting", "Ada"));
    assert!(controller.open.get_untracked());
    assert_eq!(controller.text.get_untracked(), "Hello, Ada!");
    assert!(actions.dispatch("hello", "", ""));
    assert!(!controller.open.get_untracked());
    actions.dispatch("open", "hello", "");
    assert_eq!(actions.open_popups(), vec!["greeting"]);
    actions.dispatch("close", "", "");
    assert!(actions.open_popups().is_empty());
}

#[test]
fn escape_closes_through_the_view() {
    let (actions, controller, mut scene) = rig();
    actions.dispatch("open", "greeting", "");
    assert!(scene.key_event(&KeyEvent::new(Key::Escape)));
    assert!(!controller.open.get_untracked());
}

#[test]
fn unknown_commands_are_not_handled() {
    let (actions, _controller, _scene) = rig();
    assert!(!actions.dispatch("open", "nothing", ""));
}

#[test]
fn the_shortcut_toggles_the_greeting() {
    let (actions, controller, _scene) = rig();
    let keys = acanthus_app::Keybindings::new()
        .bind(
            crate::model::SHORTCUT,
            "Toggle the greeting",
            "Super+G",
            acanthus_app::Verb::Toggle,
            crate::model::NAME,
            "",
        )
        .unwrap();
    assert!(keys.activate(&actions, crate::model::SHORTCUT));
    assert!(controller.open.get_untracked());
    assert!(keys.activate(&actions, crate::model::SHORTCUT));
    assert!(!controller.open.get_untracked());
}
