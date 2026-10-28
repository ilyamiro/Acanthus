use acanthus_services::platform::mpris::{Playback, Player};

pub const WIDTH: u32 = 320;
pub const HEIGHT: u32 = 120;
pub const MARGIN: i32 = 32;

pub fn now_playing(player: Option<&Player>) -> (String, String) {
    match player {
        Some(p) if p.playback != Playback::Stopped && !p.title.is_empty() => (p.title.clone(), p.artist.clone()),
        _ => ("Nothing playing".to_owned(), String::new()),
    }
}
