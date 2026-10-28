use std::cell::RefCell;
use std::collections::HashMap;

use acanthus_core::{Effect, Signal, untrack};

use crate::{Config, SettingKeys};

/// Interface scale per screen, from the settings.
pub struct Scaler {
    config: Config,
    keys: SettingKeys,
    scales: RefCell<HashMap<String, (Signal<f32>, Effect)>>,
}

impl Scaler {
    /// Scaler reading the scale settings named by `keys` from `config`.
    pub fn new(config: &Config, keys: &SettingKeys) -> Scaler {
        Scaler { config: config.clone(), keys: keys.clone(), scales: RefCell::default() }
    }

    /// Signal of the scale of `screen`: the screen's own setting, else the general one. Bind it with `Scene::set_ui_scale` on that screen's surfaces.
    pub fn scale_for(&self, screen: &str) -> Signal<f32> {
        if let Some((signal, _)) = self.scales.borrow().get(screen) {
            return signal.clone();
        }
        let per_monitor = self.config.value(&self.keys.screen_scale_for(screen));
        let general = self.config.value(&self.keys.ui_scale);
        let signal = Signal::new(1.0f32);
        let target = signal.clone();
        let effect = Effect::new(move || {
            let monitor = per_monitor.get().as_f64();
            let global = general.get().as_f64();
            let next = monitor.or(global).unwrap_or(1.0) as f32;
            untrack(|| target.set(next));
        });
        self.scales.borrow_mut().insert(screen.to_owned(), (signal.clone(), effect));
        signal
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn monitor_scale_beats_general_and_defaults_to_one() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("s.json");
        std::fs::write(&path, r#"{"display": {"monitors": {"eDP-1": {"scale": 1.5}}}}"#).unwrap();
        let config = Config::load_demo(&path);
        let keys = SettingKeys {
            ui_scale: "general.uiScale".into(),
            screen_scale: "display.monitors.{screen}.scale".into(),
            ..SettingKeys::default()
        };
        let scaler = Scaler::new(&config, &keys);
        let laptop = scaler.scale_for("eDP-1");
        let other = scaler.scale_for("HDMI-A-1");
        assert_eq!(laptop.get_untracked(), 1.5);
        assert_eq!(other.get_untracked(), 1.0);
        config.set("general.uiScale", json!(2.0));
        assert_eq!(other.get_untracked(), 2.0);
        assert_eq!(laptop.get_untracked(), 1.5);
        config.set("display.monitors.eDP-1.scale", json!(0.75));
        assert_eq!(laptop.get_untracked(), 0.75);
    }

    #[test]
    fn default_keys_read_neutral_paths() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("s.json");
        std::fs::write(&path, r#"{"general": {"uiScale": 3.0}}"#).unwrap();
        let config = Config::load_demo(&path);
        let scaler = Scaler::new(&config, &SettingKeys::default());
        let screen = scaler.scale_for("DP-2");
        assert_eq!(screen.get_untracked(), 1.0);
        config.set("general.ui_scale", json!(1.25));
        assert_eq!(screen.get_untracked(), 1.25);
        config.set("screens.DP-2.scale", json!(2.0));
        assert_eq!(screen.get_untracked(), 2.0);
    }
}
