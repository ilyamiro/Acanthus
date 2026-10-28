/// The compositors the service can talk to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CompositorKind {
    /// Hyprland.
    Hyprland,
    /// Niri.
    Niri,
    /// Sway.
    Sway,
    /// Mango.
    Mango,
    /// Anything else; no compositor service is started.
    #[default]
    Unknown,
}

impl CompositorKind {
    /// Detects the compositor from the instance variables, then from the desktop name variables, read through `env`.
    pub fn detect(env: impl Fn(&str) -> Option<String>) -> CompositorKind {
        let set = |key: &str| env(key).is_some_and(|v| !v.is_empty());
        if set("HYPRLAND_INSTANCE_SIGNATURE") {
            return CompositorKind::Hyprland;
        }
        if set("NIRI_SOCKET") {
            return CompositorKind::Niri;
        }
        if set("SWAYSOCK") {
            return CompositorKind::Sway;
        }
        if set("MANGO_INSTANCE_SIGNATURE") {
            return CompositorKind::Mango;
        }
        let desktop = ["XDG_CURRENT_DESKTOP", "XDG_SESSION_DESKTOP", "DESKTOP_SESSION"]
            .iter()
            .filter_map(|k| env(k))
            .collect::<Vec<_>>()
            .join(":")
            .to_lowercase();
        [
            ("hyprland", CompositorKind::Hyprland),
            ("niri", CompositorKind::Niri),
            ("sway", CompositorKind::Sway),
            ("mango", CompositorKind::Mango),
        ]
        .into_iter()
        .find(|(name, _)| desktop.contains(name))
        .map_or(CompositorKind::Unknown, |(_, kind)| kind)
    }
}

/// A workspace as the compositor reports it.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Workspace {
    /// Compositor-wide identifier.
    pub id: i64,
    /// Position in the workspace list, as shown to the user.
    pub index: i32,
    /// Name, empty when unnamed.
    pub name: String,
    /// Name of the output it is on.
    pub output: String,
    /// True when it is the visible workspace of its output.
    pub active: bool,
    /// True when it is the focused workspace.
    pub focused: bool,
    /// True when it holds windows.
    pub occupied: bool,
    /// True when a window on it asks for attention.
    pub urgent: bool,
    /// True when a window on it is fullscreen.
    pub has_fullscreen: bool,
}

/// A window as the compositor reports it.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Window {
    /// Stable identifier.
    pub id: String,
    /// Identifier in the compositor's own format.
    pub native_id: String,
    /// Application id or class.
    pub app_id: String,
    /// Title.
    pub title: String,
    /// Index of its workspace.
    pub workspace: i32,
    /// Name of its workspace.
    pub workspace_name: String,
    /// Name of its output.
    pub output: String,
    /// True when it has focus.
    pub focused: bool,
    /// True when it is fullscreen.
    pub fullscreen: bool,
    /// True when it floats.
    pub floating: bool,
    /// True when it asks for attention.
    pub urgent: bool,
    /// Process id, when the compositor tells it.
    pub pid: Option<i64>,
}

/// Everything the compositor reports at one moment.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Snapshot {
    /// All workspaces.
    pub workspaces: Vec<Workspace>,
    /// All windows.
    pub windows: Vec<Window>,
    /// Name of the focused output.
    pub focused_output: String,
    /// Names of the configured keyboard layouts.
    pub keyboard_layouts: Vec<String>,
    /// Index of the active layout.
    pub keyboard_layout_index: usize,
}

impl Snapshot {
    /// Index of the focused workspace, else the visible one, else the first; -1 when there are none.
    pub fn active_index(&self) -> i32 {
        self.workspaces
            .iter()
            .find(|w| w.focused)
            .or_else(|| self.workspaces.iter().find(|w| w.active))
            .or_else(|| self.workspaces.first())
            .map_or(-1, |w| w.index)
    }

    /// Like [`Snapshot::active_index`] restricted to the workspaces of `output`, falling back to the global answer.
    pub fn active_index_for(&self, output: &str) -> i32 {
        let on_output = || self.workspaces.iter().filter(|w| w.output == output);
        on_output()
            .find(|w| w.focused)
            .or_else(|| on_output().find(|w| w.active))
            .or_else(|| on_output().next())
            .map_or_else(|| self.active_index(), |w| w.index)
    }

    /// True when workspace `index` holds windows, judged on `output` when it has such a workspace.
    pub fn is_occupied(&self, output: &str, index: i32) -> bool {
        let on_output = self.workspaces.iter().any(|w| w.output == output && w.index == index);
        self.workspaces.iter().any(|w| (!on_output || w.output == output) && w.index == index && w.occupied)
    }

    /// The largest workspace index, zero when there are none.
    pub fn max_workspace_index(&self) -> i32 {
        self.workspaces.iter().map(|w| w.index).max().unwrap_or(0)
    }

    /// The focused window.
    pub fn focused_window(&self) -> Option<&Window> {
        self.windows.iter().find(|w| w.focused)
    }

    /// The active keyboard layout name, empty when unknown.
    pub fn keyboard_layout(&self) -> &str {
        self.keyboard_layouts.get(self.keyboard_layout_index).map_or("", String::as_str)
    }

    /// Two-letter upper-case code of the active layout, `US` when unknown.
    pub fn short_keyboard_layout(&self) -> String {
        let layout = self.keyboard_layout();
        if layout.is_empty() { "US".into() } else { layout.chars().take(2).collect::<String>().to_uppercase() }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env_of(pairs: &'static [(&'static str, &'static str)]) -> impl Fn(&str) -> Option<String> {
        move |k| pairs.iter().find(|(key, _)| *key == k).map(|(_, v)| (*v).to_owned())
    }

    #[test]
    fn detection_prefers_the_compositor_specific_variable() {
        assert_eq!(CompositorKind::detect(env_of(&[("HYPRLAND_INSTANCE_SIGNATURE", "x")])), CompositorKind::Hyprland);
        assert_eq!(CompositorKind::detect(env_of(&[("NIRI_SOCKET", "/s")])), CompositorKind::Niri);
        assert_eq!(CompositorKind::detect(env_of(&[("SWAYSOCK", "/s")])), CompositorKind::Sway);
        assert_eq!(CompositorKind::detect(env_of(&[("MANGO_INSTANCE_SIGNATURE", "m")])), CompositorKind::Mango);
        assert_eq!(CompositorKind::detect(env_of(&[("XDG_CURRENT_DESKTOP", "Sway")])), CompositorKind::Sway);
        assert_eq!(CompositorKind::detect(env_of(&[("XDG_CURRENT_DESKTOP", "KDE")])), CompositorKind::Unknown);
        assert_eq!(CompositorKind::detect(env_of(&[("NIRI_SOCKET", "")])), CompositorKind::Unknown);
    }

    fn ws(index: i32, output: &str, active: bool, focused: bool, occupied: bool) -> Workspace {
        Workspace {
            id: i64::from(index) + 1,
            index,
            output: output.into(),
            active,
            focused,
            occupied,
            ..Default::default()
        }
    }

    #[test]
    fn active_index_per_output_and_global() {
        let snapshot = Snapshot {
            workspaces: vec![
                ws(0, "A", true, false, true),
                ws(1, "A", false, false, false),
                ws(0, "B", false, false, false),
                ws(2, "B", true, true, true),
            ],
            ..Default::default()
        };
        assert_eq!(snapshot.active_index(), 2);
        assert_eq!(snapshot.active_index_for("A"), 0);
        assert_eq!(snapshot.active_index_for("B"), 2);
        assert_eq!(snapshot.active_index_for("missing"), 2);
        assert!(snapshot.is_occupied("A", 0));
        assert!(!snapshot.is_occupied("A", 1));
        assert!(snapshot.is_occupied("B", 2));
        assert_eq!(snapshot.max_workspace_index(), 2);
    }

    #[test]
    fn keyboard_layout_helpers() {
        let mut snapshot = Snapshot::default();
        assert_eq!(snapshot.short_keyboard_layout(), "US");
        snapshot.keyboard_layouts = vec!["English (US)".into(), "Ukrainian".into()];
        snapshot.keyboard_layout_index = 1;
        assert_eq!(snapshot.keyboard_layout(), "Ukrainian");
        assert_eq!(snapshot.short_keyboard_layout(), "UK");
    }
}
