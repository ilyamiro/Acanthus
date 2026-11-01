use std::path::Path;

use acanthus_anim::Keep;
use acanthus_core::Vec2;
use acanthus_runtime::I18n;
use acanthus_scene::{Fonts, Key, KeyEvent, Primitive, Scene};

use crate::controller::Controller;
use crate::model::{Note, pinned_first, title};
use crate::view::build;

fn english() -> I18n {
    I18n::new(&Path::new(env!("CARGO_MANIFEST_DIR")).join("i18n"), "en")
}

fn note(id: u32, pinned: bool) -> Note {
    Note { id, text: String::new(), pinned }
}

fn glyphs(scene: &mut Scene) -> usize {
    scene.frame().primitives.iter().filter(|p| matches!(p, Primitive::Glyph(_))).count()
}

#[test]
fn the_title_is_the_first_non_blank_line() {
    assert_eq!(title("\n  Shopping \nmilk"), Some("Shopping"));
    assert_eq!(title("  \n"), None);
}

#[test]
fn pinned_notes_keep_their_order_at_the_top() {
    let sorted = pinned_first(vec![note(1, false), note(2, true), note(3, false), note(4, true)]);
    assert_eq!(sorted.iter().map(|n| n.id).collect::<Vec<_>>(), [2, 4, 1, 3]);
}

#[test]
fn adding_editing_pinning_and_deleting_keep_selection_and_text_in_step() {
    let mut keep = Keep::default();
    let controller = Controller::new(&mut keep);
    controller.add();
    controller.text.set("first".to_owned());
    controller.add();
    controller.text.set("second".to_owned());
    assert_eq!(
        controller.notes.get_untracked().iter().map(|n| n.text.as_str()).collect::<Vec<_>>(),
        ["second", "first"]
    );
    controller.selected.set(Some(1));
    assert_eq!(controller.text.get_untracked(), "first");
    controller.set_pinned(true);
    assert_eq!(controller.notes.get_untracked()[0].id, 1);
    controller.delete();
    assert_eq!(controller.selected.get_untracked(), Some(2));
    assert_eq!(controller.text.get_untracked(), "second");
    controller.delete();
    assert_eq!(controller.selected.get_untracked(), None);
    assert_eq!(controller.text.get_untracked(), "");
}

#[test]
fn the_window_lists_titles_and_edits_the_selected_note() {
    let mut scene = Scene::new(Fonts::new());
    scene.set_viewport(Vec2::new(720.0, 440.0), 1.0);
    let mut keep = Keep::default();
    let controller = Controller::new(&mut keep);
    let _view = build(&mut scene, &controller, &english());
    let base = glyphs(&mut scene);
    controller.add();
    scene.frame();
    let untitled = glyphs(&mut scene);
    assert!(untitled > base);
    for c in "Hi".chars() {
        scene.frame();
        scene.key_event(&KeyEvent::new(Key::Char(c)));
    }
    scene.frame();
    assert_eq!(controller.current().unwrap().text, "Hi");
}
