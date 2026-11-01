pub const WIDTH: u32 = 720;
pub const HEIGHT: u32 = 440;
pub const LIST_WIDTH: f32 = 220.0;
pub const ROW: f32 = 36.0;
pub const GAP: f32 = 12.0;

#[derive(Clone, PartialEq, Debug)]
pub struct Note {
    pub id: u32,
    pub text: String,
    pub pinned: bool,
}

pub fn title(text: &str) -> Option<&str> {
    text.lines().map(str::trim).find(|line| !line.is_empty())
}

pub fn pinned_first(mut notes: Vec<Note>) -> Vec<Note> {
    notes.sort_by_key(|note| !note.pinned);
    notes
}
