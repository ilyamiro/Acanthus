/// Settings paths the runtime services read, supplied by the application so the engine carries no settings schema.
///
/// Paths are dotted, as in [`Config::value`](crate::Config::value). The defaults are neutral names; an application with
/// an existing settings file passes its own.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SettingKeys {
    /// Interface language; empty or unset follows the system language.
    pub language: String,
    /// Time format candidates, the first one set wins; none set gives [`DEFAULT_TIME_FORMAT`](crate::DEFAULT_TIME_FORMAT).
    pub time_format: Vec<String>,
    /// Theme section (preset, generated colours, radius, font).
    pub theme: String,
    /// Current wallpaper, the source image of generated colours when the theme names none.
    pub wallpaper: String,
    /// Interface scale of every screen.
    pub ui_scale: String,
    /// Interface scale of one screen; `{screen}` is replaced by the output name. Beats [`ui_scale`](Self::ui_scale).
    pub screen_scale: String,
}

impl Default for SettingKeys {
    fn default() -> Self {
        SettingKeys {
            language: "general.language".into(),
            time_format: vec!["general.time_format".into()],
            theme: "theme".into(),
            wallpaper: "wallpaper".into(),
            ui_scale: "general.ui_scale".into(),
            screen_scale: "screens.{screen}.scale".into(),
        }
    }
}

impl SettingKeys {
    /// Path of the scale setting of `screen`.
    pub fn screen_scale_for(&self, screen: &str) -> String {
        self.screen_scale.replace("{screen}", screen)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn screen_scale_template_takes_the_output_name() {
        let keys = SettingKeys { screen_scale: "display.monitors.{screen}.scale".into(), ..SettingKeys::default() };
        assert_eq!(keys.screen_scale_for("eDP-1"), "display.monitors.eDP-1.scale");
        assert_eq!(SettingKeys::default().screen_scale_for("HDMI-A-1"), "screens.HDMI-A-1.scale");
    }
}
