use acanthus_core::{Signal, Vec2};
use acanthus_scene::{Fonts, Primitive, Scene};
use acanthus_services::platform::mpris::{Playback, Player};

use crate::model::now_playing;
use crate::view::{Inputs, build};

fn glyphs(scene: &mut Scene) -> usize {
    scene.frame().primitives.iter().filter(|p| matches!(p, Primitive::Glyph(_))).count()
}

fn player(playback: Playback, title: &str) -> Player {
    Player { playback, title: title.to_owned(), artist: "Band".to_owned(), ..Player::default() }
}

#[test]
fn track_shows_only_while_something_plays() {
    assert_eq!(now_playing(None).0, "Nothing playing");
    assert_eq!(now_playing(Some(&player(Playback::Stopped, "Song"))).0, "Nothing playing");
    assert_eq!(now_playing(Some(&player(Playback::Playing, ""))).0, "Nothing playing");
    assert_eq!(now_playing(Some(&player(Playback::Paused, "Song"))), ("Song".to_owned(), "Band".to_owned()));
}

#[test]
fn widget_text_follows_the_signals() {
    let mut scene = Scene::new(Fonts::new());
    scene.set_viewport(Vec2::new(320.0, 120.0), 1.0);
    let inputs = Inputs {
        time: Signal::new("12:34".to_owned()),
        title: Signal::new(String::new()),
        artist: Signal::new(String::new()),
    };
    let (title, artist) = (inputs.title.clone(), inputs.artist.clone());
    build(&mut scene, inputs);
    let base = glyphs(&mut scene);
    title.set("Song".to_owned());
    artist.set("Band".to_owned());
    assert_eq!(glyphs(&mut scene), base + 8);
}
