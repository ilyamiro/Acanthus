use std::fmt;

use acanthus_core::Vec2;
use acanthus_scene::{Key, KeyEvent, Modifiers, MouseButton, ScrollDelta};

/// One thing a [`Script`] does to the scene.
#[derive(Debug, Clone, PartialEq)]
pub enum Action {
    /// Moves the pointer to a position in logical pixels.
    Move(Vec2),
    /// Presses a pointer button.
    Down(MouseButton),
    /// Releases a pointer button.
    Up(MouseButton),
    /// Presses and releases a pointer button at once.
    Click(MouseButton),
    /// The pointer leaves the surface.
    Leave,
    /// Scrolls by a delta.
    Scroll(ScrollDelta),
    /// Delivers a key press.
    Key(KeyEvent),
    /// Blocks the thread for this many milliseconds, as a slow handler would.
    Stall(u32),
}

/// An [`Action`] at a point of the virtual clock.
#[derive(Debug, Clone, PartialEq)]
pub struct Step {
    /// Virtual time in milliseconds.
    pub at_ms: u32,
    /// What happens then.
    pub action: Action,
}

/// Why a script text could not be read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScriptError {
    /// The offending token.
    pub token: String,
    /// What is wrong with it.
    pub reason: &'static str,
}

impl fmt::Display for ScriptError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "script token `{}`: {}", self.token, self.reason)
    }
}

impl std::error::Error for ScriptError {}

/// A timeline of pointer, key and stall steps replayed on a deterministic clock.
///
/// The text form is whitespace separated `action@milliseconds` tokens: `x,y` moves the pointer,
/// `click`, `rclick`, `down`, `up` and `leave` act on the pointer, `scroll:dx,dy` scrolls,
/// `key:Escape` or `key:ctrl+shift+a` presses a key, `stall:600` blocks the thread for 600 ms.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Script {
    steps: Vec<Step>,
}

impl Script {
    /// A script without steps.
    pub fn new() -> Script {
        Script::default()
    }

    /// Reads the text form.
    pub fn parse(text: &str) -> Result<Script, ScriptError> {
        let mut steps = Vec::new();
        for token in text.split_whitespace() {
            let fail = |reason| ScriptError { token: token.to_owned(), reason };
            let (action, at) = token.rsplit_once('@').ok_or_else(|| fail("missing `@milliseconds`"))?;
            let at_ms = at.parse::<u32>().map_err(|_| fail("the time is not whole milliseconds"))?;
            steps.push(Step { at_ms, action: parse_action(action).map_err(fail)? });
        }
        steps.sort_by_key(|step| step.at_ms);
        Ok(Script { steps })
    }

    /// Adds `action` at `at_ms`, keeping the steps in time order.
    pub fn at(mut self, at_ms: u32, action: Action) -> Script {
        let index = self.steps.partition_point(|step| step.at_ms <= at_ms);
        self.steps.insert(index, Step { at_ms, action });
        self
    }

    /// The steps in time order.
    pub fn steps(&self) -> &[Step] {
        &self.steps
    }

    /// The time of the last step, 0 for an empty script.
    pub fn end_ms(&self) -> u32 {
        self.steps.last().map_or(0, |step| step.at_ms)
    }
}

fn parse_pair(text: &str) -> Option<(f32, f32)> {
    let (x, y) = text.split_once(',')?;
    Some((x.trim().parse().ok()?, y.trim().parse().ok()?))
}

fn parse_action(text: &str) -> Result<Action, &'static str> {
    match text {
        "click" => return Ok(Action::Click(MouseButton::Left)),
        "rclick" => return Ok(Action::Click(MouseButton::Right)),
        "down" => return Ok(Action::Down(MouseButton::Left)),
        "up" => return Ok(Action::Up(MouseButton::Left)),
        "leave" => return Ok(Action::Leave),
        _ => {}
    }
    if let Some(rest) = text.strip_prefix("scroll:") {
        let (x, y) = parse_pair(rest).ok_or("scroll wants `dx,dy`")?;
        return Ok(Action::Scroll(ScrollDelta { x, y }));
    }
    if let Some(rest) = text.strip_prefix("stall:") {
        return rest.parse().map(Action::Stall).map_err(|_| "stall wants whole milliseconds");
    }
    if let Some(rest) = text.strip_prefix("key:") {
        return parse_key(rest).map(Action::Key);
    }
    let (x, y) = parse_pair(text).ok_or("unknown action")?;
    Ok(Action::Move(Vec2::new(x, y)))
}

fn parse_key(text: &str) -> Result<KeyEvent, &'static str> {
    let mut modifiers = Modifiers::default();
    let mut parts: Vec<&str> = text.split('+').collect();
    let name = parts.pop().filter(|name| !name.is_empty()).ok_or("key wants a name")?;
    for part in parts {
        match part.to_ascii_lowercase().as_str() {
            "ctrl" => modifiers.ctrl = true,
            "shift" => modifiers.shift = true,
            "alt" => modifiers.alt = true,
            "logo" | "super" => modifiers.logo = true,
            _ => return Err("unknown modifier"),
        }
    }
    let key = match name.to_ascii_lowercase().as_str() {
        "enter" | "return" => Key::Enter,
        "escape" | "esc" => Key::Escape,
        "backspace" => Key::Backspace,
        "delete" => Key::Delete,
        "tab" => Key::Tab,
        "left" => Key::Left,
        "right" => Key::Right,
        "up" => Key::Up,
        "down" => Key::Down,
        "home" => Key::Home,
        "end" => Key::End,
        "pageup" => Key::PageUp,
        "pagedown" => Key::PageDown,
        "insert" => Key::Insert,
        "space" => Key::Char(' '),
        other => match other.strip_prefix('f').and_then(|n| n.parse::<u8>().ok()) {
            Some(n) if (1..=24).contains(&n) => Key::Function(n),
            _ => {
                let mut chars = name.chars();
                match (chars.next(), chars.next()) {
                    (Some(c), None) => Key::Char(c),
                    _ => return Err("unknown key"),
                }
            }
        },
    };
    let mut event = KeyEvent::new(key);
    event.modifiers = modifiers;
    Ok(event)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_documented_example_parses_in_time_order() {
        let script = Script::parse("360,90@500 click@800 key:Escape@1200").unwrap();
        assert_eq!(script.steps().len(), 3);
        assert_eq!(script.steps()[0], Step { at_ms: 500, action: Action::Move(Vec2::new(360.0, 90.0)) });
        assert_eq!(script.steps()[1].action, Action::Click(MouseButton::Left));
        assert_eq!(script.steps()[2].action, Action::Key(KeyEvent::new(Key::Escape)));
        assert_eq!(script.end_ms(), 1200);
    }

    #[test]
    fn steps_sort_stably_by_time() {
        let script = Script::parse("click@50 down@10 up@50 leave@10").unwrap();
        let order: Vec<_> = script.steps().iter().map(|s| s.action.clone()).collect();
        assert_eq!(
            order,
            vec![
                Action::Down(MouseButton::Left),
                Action::Leave,
                Action::Click(MouseButton::Left),
                Action::Up(MouseButton::Left)
            ]
        );
    }

    #[test]
    fn keys_carry_modifiers_and_characters() {
        let script = Script::parse("key:ctrl+shift+a@0 key:F5@1 key:space@2 stall:600@3 scroll:0,-120@4").unwrap();
        let Action::Key(first) = &script.steps()[0].action else { panic!() };
        assert_eq!(first.key, Key::Char('a'));
        assert!(first.modifiers.ctrl && first.modifiers.shift && !first.modifiers.alt);
        assert!(matches!(&script.steps()[1].action, Action::Key(e) if e.key == Key::Function(5)));
        assert!(matches!(&script.steps()[2].action, Action::Key(e) if e.key == Key::Char(' ')));
        assert_eq!(script.steps()[3].action, Action::Stall(600));
        assert_eq!(script.steps()[4].action, Action::Scroll(ScrollDelta { x: 0.0, y: -120.0 }));
    }

    #[test]
    fn bad_tokens_are_named() {
        assert_eq!(Script::parse("click").unwrap_err().reason, "missing `@milliseconds`");
        assert_eq!(Script::parse("click@soon").unwrap_err().reason, "the time is not whole milliseconds");
        assert_eq!(Script::parse("hop@1").unwrap_err().reason, "unknown action");
        assert_eq!(Script::parse("key:hyper+a@1").unwrap_err().reason, "unknown modifier");
        assert_eq!(Script::parse("key:nope@1").unwrap_err().to_string(), "script token `key:nope@1`: unknown key");
    }

    #[test]
    fn builder_keeps_time_order() {
        let script = Script::new().at(30, Action::Leave).at(10, Action::Stall(1)).at(30, Action::Stall(2));
        let times: Vec<_> = script.steps().iter().map(|s| (s.at_ms, s.action.clone())).collect();
        assert_eq!(times, vec![(10, Action::Stall(1)), (30, Action::Leave), (30, Action::Stall(2))]);
    }
}
