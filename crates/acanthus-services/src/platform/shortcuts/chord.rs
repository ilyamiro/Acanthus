use std::fmt;
use std::str::FromStr;

/// The modifier keys of a chord.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Modifiers {
    /// Shift.
    pub shift: bool,
    /// Control.
    pub ctrl: bool,
    /// Alt.
    pub alt: bool,
    /// The logo key (Super, Windows, Command).
    pub logo: bool,
}

/// A chord that did not parse.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ChordError {
    /// The text was empty.
    #[error("empty chord")]
    Empty,
    /// A part before the last one is not a modifier name.
    #[error("unknown modifier `{0}`")]
    UnknownModifier(String),
    /// The chord ends in a modifier or has no key.
    #[error("chord `{0}` has no key")]
    NoKey(String),
}

/// A key combination such as `Super+Shift+K`, written once and rendered in the syntax of each compositor and of the shortcuts portal.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Chord {
    /// Held modifiers.
    pub modifiers: Modifiers,
    /// The key as an xkb keysym name (`K`, `Return`, `F5`, `XF86AudioPlay`); a single letter is stored lowercase.
    pub key: String,
}

fn modifier(word: &str) -> Option<fn(&mut Modifiers)> {
    match word.to_ascii_lowercase().as_str() {
        "shift" => Some(|m| m.shift = true),
        "ctrl" | "control" | "ctl" => Some(|m| m.ctrl = true),
        "alt" | "mod1" | "option" => Some(|m| m.alt = true),
        "super" | "logo" | "mod4" | "meta" | "win" | "windows" | "cmd" | "command" | "mod" => Some(|m| m.logo = true),
        _ => None,
    }
}

impl FromStr for Chord {
    type Err = ChordError;

    fn from_str(text: &str) -> Result<Chord, ChordError> {
        let trimmed = text.trim();
        if trimmed.is_empty() {
            return Err(ChordError::Empty);
        }
        let (head, plus) = match trimmed.strip_suffix("++") {
            Some(head) => (head, true),
            None => (trimmed, false),
        };
        let mut parts: Vec<&str> = head.split('+').map(str::trim).collect();
        if plus {
            parts.push("plus");
        }
        let key = parts.pop().filter(|k| !k.is_empty()).ok_or_else(|| ChordError::NoKey(trimmed.to_owned()))?;
        if modifier(key).is_some() {
            return Err(ChordError::NoKey(trimmed.to_owned()));
        }
        let mut modifiers = Modifiers::default();
        for word in parts {
            let apply = modifier(word).ok_or_else(|| ChordError::UnknownModifier(word.to_owned()))?;
            apply(&mut modifiers);
        }
        let key = if key.chars().count() == 1 { key.to_lowercase() } else { key.to_owned() };
        Ok(Chord { modifiers, key })
    }
}

impl Chord {
    /// Parses a chord; modifiers are case-insensitive (`Super`, `Mod4`, `Ctrl`, `Alt`, `Shift` and common aliases).
    pub fn parse(text: &str) -> Result<Chord, ChordError> {
        text.parse()
    }

    fn names(
        &self,
        logo: &'static str,
        ctrl: &'static str,
        alt: &'static str,
        shift: &'static str,
    ) -> Vec<&'static str> {
        let m = self.modifiers;
        [(m.logo, logo), (m.ctrl, ctrl), (m.alt, alt), (m.shift, shift)]
            .into_iter()
            .filter_map(|(held, name)| held.then_some(name))
            .collect()
    }

    fn joined(&self, parts: Vec<&'static str>, key: &str) -> String {
        parts.into_iter().map(str::to_owned).chain(std::iter::once(key.to_owned())).collect::<Vec<_>>().join("+")
    }

    fn display_key(&self) -> String {
        if self.key.chars().count() == 1 { self.key.to_uppercase() } else { self.key.clone() }
    }

    /// The `MODS, key` part of a Hyprland `bind` line (`SUPER SHIFT, K`).
    pub fn hyprland(&self) -> String {
        format!("{}, {}", self.names("SUPER", "CTRL", "ALT", "SHIFT").join(" "), self.display_key())
    }

    /// The `MODS,key` part of a Mango `bind` line (`SUPER+SHIFT,k`, `NONE,F5`).
    pub fn mango(&self) -> String {
        let names = self.names("SUPER", "CTRL", "ALT", "SHIFT");
        let mods = if names.is_empty() { "NONE".to_owned() } else { names.join("+") };
        format!("{mods},{}", self.key)
    }

    /// The key combination of a Sway `bindsym` (`Mod4+Shift+k`).
    pub fn sway(&self) -> String {
        self.joined(self.names("Mod4", "Ctrl", "Mod1", "Shift"), &self.key)
    }

    /// The key combination of a Niri `binds` entry (`Mod+Shift+K`).
    pub fn niri(&self) -> String {
        self.joined(self.names("Mod", "Ctrl", "Alt", "Shift"), &self.display_key())
    }

    /// The `preferred_trigger` of the XDG GlobalShortcuts portal (`LOGO+SHIFT+k`).
    pub fn portal(&self) -> String {
        self.joined(self.names("LOGO", "CTRL", "ALT", "SHIFT"), &self.key)
    }
}

impl fmt::Display for Chord {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.joined(self.names("Super", "Ctrl", "Alt", "Shift"), &self.display_key()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_modifiers_and_aliases_in_any_case() {
        let chord = Chord::parse("mod4 + SHIFT+k").unwrap();
        assert!(chord.modifiers.logo && chord.modifiers.shift && !chord.modifiers.ctrl);
        assert_eq!(chord.key, "k");
        assert_eq!(Chord::parse("Control+Alt+Delete").unwrap().key, "Delete");
    }

    #[test]
    fn renders_each_syntax() {
        let chord = Chord::parse("Super+Ctrl+Shift+K").unwrap();
        assert_eq!(chord.hyprland(), "SUPER CTRL SHIFT, K");
        assert_eq!(chord.sway(), "Mod4+Ctrl+Shift+k");
        assert_eq!(chord.niri(), "Mod+Ctrl+Shift+K");
        assert_eq!(chord.portal(), "LOGO+CTRL+SHIFT+k");
        assert_eq!(chord.to_string(), "Super+Ctrl+Shift+K");
    }

    #[test]
    fn named_keys_keep_their_case() {
        let chord = Chord::parse("Alt+Return").unwrap();
        assert_eq!(chord.sway(), "Mod1+Return");
        assert_eq!(chord.hyprland(), "ALT, Return");
        assert_eq!(Chord::parse("XF86AudioPlay").unwrap().hyprland(), ", XF86AudioPlay");
    }

    #[test]
    fn a_trailing_plus_is_the_plus_key() {
        let chord = Chord::parse("Ctrl++").unwrap();
        assert!(chord.modifiers.ctrl);
        assert_eq!(chord.key, "plus");
    }

    #[test]
    fn rejects_bad_chords() {
        assert_eq!(Chord::parse(" "), Err(ChordError::Empty));
        assert_eq!(Chord::parse("Hyper+K"), Err(ChordError::UnknownModifier("Hyper".into())));
        assert!(matches!(Chord::parse("Super+Shift"), Err(ChordError::NoKey(_))));
        assert!(matches!(Chord::parse("Super+"), Err(ChordError::NoKey(_))));
    }

    #[test]
    fn display_round_trips() {
        let chord = Chord::parse("alt+shift+F5").unwrap();
        assert_eq!(Chord::parse(&chord.to_string()).unwrap(), chord);
    }
}
