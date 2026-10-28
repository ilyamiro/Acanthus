#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CompositorKind {
    Hyprland,
    Niri,
    Sway,
    Mango,
    #[default]
    Unknown,
}

impl CompositorKind {
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

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Workspace {
    pub id: i64,
    pub index: i32,
    pub name: String,
    pub output: String,
    pub active: bool,
    pub focused: bool,
    pub occupied: bool,
    pub urgent: bool,
    pub has_fullscreen: bool,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Window {
    pub id: String,
    pub native_id: String,
    pub app_id: String,
    pub title: String,
    pub workspace: i32,
    pub workspace_name: String,
    pub output: String,
    pub focused: bool,
    pub fullscreen: bool,
    pub floating: bool,
    pub urgent: bool,
    pub pid: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Snapshot {
    pub workspaces: Vec<Workspace>,
    pub windows: Vec<Window>,
    pub focused_output: String,
    pub keyboard_layouts: Vec<String>,
    pub keyboard_layout_index: usize,
}

impl Snapshot {
    pub fn active_index(&self) -> i32 {
        self.workspaces
            .iter()
            .find(|w| w.focused)
            .or_else(|| self.workspaces.iter().find(|w| w.active))
            .or_else(|| self.workspaces.first())
            .map_or(-1, |w| w.index)
    }

    pub fn active_index_for(&self, output: &str) -> i32 {
        let on_output = || self.workspaces.iter().filter(|w| w.output == output);
        on_output()
            .find(|w| w.focused)
            .or_else(|| on_output().find(|w| w.active))
            .or_else(|| on_output().next())
            .map_or_else(|| self.active_index(), |w| w.index)
    }

    pub fn is_occupied(&self, output: &str, index: i32) -> bool {
        let on_output = self.workspaces.iter().any(|w| w.output == output && w.index == index);
        self.workspaces.iter().any(|w| (!on_output || w.output == output) && w.index == index && w.occupied)
    }

    pub fn max_workspace_index(&self) -> i32 {
        self.workspaces.iter().map(|w| w.index).max().unwrap_or(0)
    }

    pub fn focused_window(&self) -> Option<&Window> {
        self.windows.iter().find(|w| w.focused)
    }

    pub fn keyboard_layout(&self) -> &str {
        self.keyboard_layouts.get(self.keyboard_layout_index).map_or("", String::as_str)
    }

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
