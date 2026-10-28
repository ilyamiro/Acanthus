pub const STACK: &str = "stack";
pub const NAME: &str = "greeting";
pub const WIDTH: f32 = 360.0;
pub const HEIGHT: f32 = 120.0;
pub const LINGER_MS: u64 = 150;

pub fn message(arg: &str) -> String {
    let who = arg.trim();
    if who.is_empty() { "Hello!".to_owned() } else { format!("Hello, {who}!") }
}
