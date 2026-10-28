use std::cell::RefCell;
use std::collections::HashMap;

use acanthus_core::{Effect, Signal, untrack};

use crate::Config;

/// Interface scale per screen, from the settings.
pub struct Scaler {
    config: Config,
    scales: RefCell<HashMap<String, (Signal<f32>, Effect)>>,
}

impl Scaler {
    /// Scaler reading `config`.
    pub fn new(config: &Config) -> Scaler {
        Scaler { config: config.clone(), scales: RefCell::default() }
    }

    /// Signal of the scale of `screen`: the screen's own setting, else the general one. Bind it with `Scene::set_ui_scale` on that screen's surfaces.
    pub fn scale_for(&self, screen: &str) -> Signal<f32> {
        if let Some((signal, _)) = self.scales.borrow().get(screen) {
            return signal.clone();
        }
        let per_monitor = self.config.value(&format!("display.monitors.{screen}.scale"));
        let general = self.config.value("general.uiScale");
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
        let scaler = Scaler::new(&config);
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
}
