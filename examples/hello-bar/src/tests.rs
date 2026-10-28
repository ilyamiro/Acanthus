use acanthus_core::{Signal, Vec2};
use acanthus_scene::{Fonts, Primitive, Scene};

use crate::model::battery_label;
use crate::view::{Inputs, build};

fn glyphs(scene: &mut Scene) -> usize {
    scene.frame().primitives.iter().filter(|p| matches!(p, Primitive::Glyph(_))).count()
}

#[test]
fn battery_label_hides_without_a_battery() {
    assert_eq!(battery_label(false, 80.0, false), "");
}

#[test]
fn battery_label_rounds_and_marks_charging() {
    assert_eq!(battery_label(true, 79.6, false), "80%");
    assert_eq!(battery_label(true, 100.4, true), "100% +");
    assert_eq!(battery_label(true, -3.0, false), "0%");
}

#[test]
fn bar_text_follows_the_signals() {
    let mut scene = Scene::new(Fonts::new());
    scene.set_viewport(Vec2::new(800.0, 32.0), 1.0);
    let (time, battery) = (Signal::new("12:34".to_owned()), Signal::new(String::new()));
    build(&mut scene, Inputs { time: time.clone(), battery: battery.clone() });
    let base = glyphs(&mut scene);
    assert!(base > 0);
    battery.set("87%".to_owned());
    assert_eq!(glyphs(&mut scene), base + 3);
    time.set(String::new());
    assert_eq!(glyphs(&mut scene), base + 3 - 5);
}
