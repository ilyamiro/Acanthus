use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::{Rc, Weak};

use acanthus_core::{Color, Effect, Signal, batch};
use acanthus_io::Watch;
use acanthus_scene::Fonts;
use calloop::LoopHandle;
use serde_json::Value;

use crate::font_catalog::FontCatalog;
use crate::{Config, Paths, SettingKeys, Tokens, lenient};

const ICON_FONT_FILE: &str = "SymbolsNerdFontMono-Regular.ttf";

macro_rules! palette {
    ($($name:ident = $hex:expr),* $(,)?) => {
        /// The named colour slots of a theme.
        pub struct Palette {
            $(#[doc = concat!("Colour slot `", stringify!($name), "`.")] pub $name: Signal<Color>,)*
        }

        impl Palette {
            /// Names of the palette slots.
            pub const NAMES: &'static [&'static str] = &[$(stringify!($name)),*];

            /// Palette with the default colours.
            pub fn new() -> Self {
                Palette { $($name: Signal::new(Color::hex($hex)),)* }
            }

            /// Signal of the slot called `name`.
            pub fn slot(&self, name: &str) -> Option<&Signal<Color>> {
                match name {
                    $(stringify!($name) => Some(&self.$name),)*
                    _ => None,
                }
            }
        }
    };
}

palette! {
    base = 0x1e1e2e,
    mantle = 0x181825,
    crust = 0x11111b,
    text = 0xcdd6f4,
    subtext0 = 0xa6adc8,
    subtext1 = 0xbac2de,
    surface0 = 0x313244,
    surface1 = 0x45475a,
    surface2 = 0x585b70,
    overlay0 = 0x6c7086,
    overlay1 = 0x7f849c,
    overlay2 = 0x9399b2,
    blue = 0x89b4fa,
    sapphire = 0x74c7ec,
    peach = 0xfab387,
    green = 0xa6e3a1,
    red = 0xf38ba8,
    mauve = 0xcba6f7,
    pink = 0xf5c2e7,
    yellow = 0xf9e2af,
    maroon = 0xeba0ac,
    teal = 0x94e2d5,
}

impl Default for Palette {
    fn default() -> Self {
        Palette::new()
    }
}

/// The visual style of an application: palette, fonts, radius and opacity as signals.
pub struct Theme {
    /// Colour palette.
    pub palette: Palette,
    /// Main text colour.
    pub text: Signal<Color>,
    /// Secondary text colour.
    pub subtext0: Signal<Color>,
    /// Tertiary text colour.
    pub subtext1: Signal<Color>,
    /// Accent colour.
    pub primary: Signal<Color>,
    /// Font family of the interface.
    pub font_family: Signal<String>,
    /// Corner radius as configured.
    pub border_radius: Signal<i64>,
    /// Corner radius softened for large values, see [`clamp_border_radius`].
    pub clamped_border_radius: Signal<i64>,
    /// Opacity of element backgrounds from 0.0 to 1.0.
    pub element_opacity: Signal<f32>,
    /// Whether the background is blurred.
    pub blur_background: Signal<bool>,
    /// Whether the glass effect is on.
    pub glass_active: Signal<bool>,
    /// Family of the icon font.
    pub icon_font: Signal<String>,
    /// Family of the loaded icon font file, if any.
    pub icon_family: Signal<Option<String>>,
    /// Whether the application runs without a compositor background to blur.
    pub standalone: Signal<bool>,
    /// Design scales: motion durations, spacing and radii.
    pub tokens: Tokens,
    /// Reduced motion from the settings.
    pub reduced_motion_setting: Signal<bool>,
    /// Reduced motion forced by the application, for example from a command-line flag.
    pub reduced_motion_forced: Signal<bool>,
    /// Whether reduced motion is in effect: the setting or the forced switch. It drives [`acanthus_anim::set_reduced_motion`].
    pub reduced_motion: Signal<bool>,
    effects: RefCell<Vec<Effect>>,
    sources: RefCell<Sources>,
}

#[derive(Default)]
struct Sources {
    matugen: Option<Value>,
    themes_dir: PathBuf,
    watch: Option<Watch>,
    catalog: FontCatalog,
    loaded_fonts: Vec<String>,
}

/// Softens radii above 24 so large values stay round.
pub fn clamp_border_radius(radius: i64) -> i64 {
    if radius <= 24 { radius } else { (24.0 + ((radius - 24) as f64).powf(0.55)).floor() as i64 }
}

/// Whether a theme object asks for generated colours.
pub fn is_matugen_theme(theme: &Value) -> bool {
    if let Some(flag) = theme.get("matugen") {
        return flag.as_bool() == Some(true);
    }
    let is = |key: &str, expected: &str| theme.get(key).and_then(Value::as_str) == Some(expected);
    is("theme", "matugen")
        || is("name", "matugen")
        || is("currentTheme", "matugen")
        || is("current", "matugen")
        || is("activePreset", "Matugen")
        || theme.get("activePreset").is_none()
}

fn resolve_hex(value: &Value) -> Option<String> {
    match value {
        Value::String(s) if !s.is_empty() => Some(s.clone()),
        Value::Object(o) => o
            .get("hex")
            .or_else(|| o.get("color"))
            .or_else(|| o.get("default").and_then(|d| d.get("hex")))
            .and_then(Value::as_str)
            .map(str::to_owned),
        _ => None,
    }
}

fn parse_color(text: &str) -> Option<Color> {
    let trimmed = text.trim();
    if trimmed.starts_with('#') {
        return Color::parse(trimmed);
    }
    Color::parse(&format!("#{trimmed}"))
}

fn unwrap_colors(value: &Value) -> &Value {
    value.get("colors").unwrap_or(value)
}

/// Names of the presets in `themes_dir`.
pub fn preset_names(themes_dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(themes_dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| e.file_name().to_str()?.strip_suffix(".json").map(str::to_owned))
        .collect();
    names.sort_by_key(|n| n.to_lowercase());
    names
}

/// Colours of the preset `name` in `themes_dir`.
pub fn load_preset(themes_dir: &Path, name: &str) -> Option<Value> {
    let text = std::fs::read_to_string(themes_dir.join(format!("{name}.json"))).ok()?;
    serde_json::from_str::<Value>(&text).ok().map(|v| unwrap_colors(&v).clone())
}

impl Theme {
    /// Base colour with the element opacity.
    pub fn base_fill(&self) -> Color {
        self.palette.base.get().with_alpha(self.element_opacity.get())
    }

    /// Surface colour with the element opacity.
    pub fn surface_fill(&self) -> Color {
        self.palette.surface0.get().with_alpha(self.element_opacity.get())
    }

    /// The configured radius, or `fallback` when none is set.
    pub fn radius_or(&self, fallback: f32) -> f32 {
        let configured = self.border_radius.get();
        if configured != 0 { configured as f32 } else { fallback }
    }

    /// Theme with the default palette.
    pub fn new() -> Rc<Theme> {
        let palette = Palette::new();
        let theme = Rc::new(Theme {
            text: Signal::new(Color::hex(0xcdd6f4)),
            subtext0: Signal::new(Color::hex(0xa6adc8)),
            subtext1: Signal::new(Color::hex(0xbac2de)),
            primary: Signal::new(Color::hex(0x89b4fa)),
            font_family: Signal::new("Adwaita Mono".into()),
            border_radius: Signal::new(8),
            clamped_border_radius: Signal::new(8),
            element_opacity: Signal::new(1.0),
            blur_background: Signal::new(false),
            glass_active: Signal::new(false),
            icon_font: Signal::new("Adwaita Mono".into()),
            icon_family: Signal::new(None),
            standalone: Signal::new(false),
            tokens: Tokens::default(),
            reduced_motion_setting: Signal::new(false),
            reduced_motion_forced: Signal::new(false),
            reduced_motion: Signal::new(false),
            effects: RefCell::new(Vec::new()),
            sources: RefCell::new(Sources::default()),
            palette,
        });
        theme.wire_derived();
        theme
    }

    fn wire_derived(&self) {
        let mut effects = Vec::new();
        let (glass, blur, opacity) =
            (self.glass_active.clone(), self.blur_background.clone(), self.element_opacity.clone());
        effects.push(Effect::new(move || {
            let next = blur.get() && opacity.get() < 0.999;
            acanthus_core::untrack(|| glass.set(next));
        }));
        let (clamped, radius) = (self.clamped_border_radius.clone(), self.border_radius.clone());
        effects.push(Effect::new(move || {
            let next = clamp_border_radius(radius.get());
            acanthus_core::untrack(|| clamped.set(next));
        }));
        for (target, raw) in [
            (&self.text, &self.palette.text),
            (&self.subtext0, &self.palette.subtext0),
            (&self.subtext1, &self.palette.subtext1),
        ] {
            let (target, raw) = (target.clone(), raw.clone());
            effects.push(Effect::new(move || {
                let next = raw.get();
                acanthus_core::untrack(|| target.set(next));
            }));
        }
        let (primary, blue) = (self.primary.clone(), self.palette.blue.clone());
        effects.push(Effect::new(move || {
            let next = blue.get();
            acanthus_core::untrack(|| primary.set(next));
        }));
        let (icon, icon_family, family) = (self.icon_font.clone(), self.icon_family.clone(), self.font_family.clone());
        effects.push(Effect::new(move || {
            let next = icon_family.get().unwrap_or_else(|| family.get());
            acanthus_core::untrack(|| icon.set(next));
        }));
        let (reduced, setting, forced) =
            (self.reduced_motion.clone(), self.reduced_motion_setting.clone(), self.reduced_motion_forced.clone());
        effects.push(Effect::new(move || {
            let next = setting.get() || forced.get();
            acanthus_core::untrack(|| reduced.set(next));
        }));
        let reduced = self.reduced_motion.clone();
        effects.push(Effect::new(move || acanthus_anim::set_reduced_motion(reduced.get())));
        *self.effects.borrow_mut() = effects;
    }

    /// `color` with the element opacity.
    pub fn faded(&self, color: Color) -> Color {
        color.with_alpha(self.element_opacity.get())
    }

    /// Applies the colours of a palette object; returns how many slots changed.
    pub fn apply_colors(&self, colors: &Value) -> usize {
        let Some(map) = unwrap_colors(colors).as_object() else { return 0 };
        let mut changed = 0;
        batch(|| {
            for name in Palette::NAMES {
                let Some(next) = map.get(*name).and_then(resolve_hex).and_then(|h| parse_color(&h)) else { continue };
                if let Some(slot) = self.palette.slot(name)
                    && slot.get_untracked() != next
                {
                    slot.set(next);
                    changed += 1;
                }
            }
        });
        changed
    }

    /// Applies a theme settings object: font, radius, opacity and colours.
    pub fn apply_config(&self, theme: &Value) {
        let standalone = self.standalone.get_untracked();
        batch(|| {
            if let Some(family) = theme.get("fontFamily").and_then(Value::as_str) {
                self.font_family.set(family.to_owned());
            }
            if let Some(radius) = theme.get("borderRadius").and_then(Value::as_f64) {
                self.border_radius.set(radius as i64);
            }
            let blur = !standalone && theme.get("blurBackground").and_then(Value::as_bool).unwrap_or(false);
            let percent = theme.get("elementOpacity").and_then(Value::as_f64).unwrap_or(100.0);
            let opacity = if standalone || percent.is_nan() { 1.0 } else { (percent / 100.0).clamp(0.0, 1.0) as f32 };
            self.blur_background.set(blur);
            self.element_opacity.set(opacity);
            if let Some(colors) = self.resolve_colors(theme) {
                self.apply_colors(&colors);
            }
        });
    }

    fn resolve_colors(&self, theme: &Value) -> Option<Value> {
        if is_matugen_theme(theme) {
            return self.sources.borrow().matugen.clone();
        }
        let custom = theme
            .get("colors")
            .or_else(|| theme.get("palette"))
            .or_else(|| theme.get("base").map(|_| theme))
            .filter(|v| v.is_object());
        if let Some(colors) = custom {
            return Some(colors.clone());
        }
        let preset = theme.get("activePreset").and_then(Value::as_str)?;
        load_preset(&self.sources.borrow().themes_dir, preset)
    }

    /// Sets the generated colours used when the theme asks for them.
    pub fn set_matugen_colors(&self, colors: Option<Value>) {
        self.sources.borrow_mut().matugen = colors.map(|c| unwrap_colors(&c).clone());
    }

    /// Sets the font catalog.
    pub fn set_catalog(&self, catalog: FontCatalog) {
        self.sources.borrow_mut().catalog = catalog;
    }

    /// Names of the available font families.
    pub fn font_names(&self) -> Vec<String> {
        self.sources.borrow().catalog.names().to_vec()
    }
}

/// Keeps a [`Theme`] up to date with the settings and the generated colours.
pub struct ThemeService {
    /// The theme.
    pub theme: Rc<Theme>,
    bindings: Vec<Effect>,
}

impl ThemeService {
    /// Starts watching the theme section named by `keys` and the generated colour file.
    pub fn start<D: 'static>(
        config: &Config,
        keys: &SettingKeys,
        paths: &Paths,
        fonts: &Fonts,
        handle: &LoopHandle<'static, D>,
    ) -> Result<ThemeService, acanthus_io::IoError> {
        let theme = Theme::new();
        let font_dirs = [paths.assets.join("fonts"), paths.state_dir.join("fonts")];
        {
            let mut sources = theme.sources.borrow_mut();
            sources.themes_dir = paths.themes_dir();
            sources.catalog = FontCatalog::scan(&font_dirs);
        }
        if let Some(family) = fonts.load_file(&paths.assets.join("fonts").join(ICON_FONT_FILE)).into_iter().next() {
            theme.icon_family.set(Some(family));
        }
        let matugen_path = paths.state_dir.join("matugen_colors.json");
        let load_matugen = {
            let weak: Weak<Theme> = Rc::downgrade(&theme);
            let path = matugen_path.clone();
            let (config, theme_key) = (config.clone(), keys.theme.clone());
            move || {
                let Some(theme) = weak.upgrade() else { return };
                let Ok(text) = std::fs::read_to_string(&path) else { return };
                let Ok(value) = serde_json::from_str::<Value>(text.trim()) else { return };
                theme.set_matugen_colors(Some(value));
                theme.apply_config(&config.current(&theme_key));
            }
        };
        let mut bindings = Vec::new();
        let reduced_value = config.value(&keys.reduced_motion);
        let weak_theme = Rc::downgrade(&theme);
        bindings.push(Effect::new(move || {
            let on = lenient::is_on(&reduced_value.get());
            if let Some(theme) = weak_theme.upgrade() {
                theme.reduced_motion_setting.set(on);
            }
        }));
        let theme_value = config.value(&keys.theme);
        let (weak, fonts_for_effect) = (Rc::downgrade(&theme), fonts.clone());
        load_matugen();
        bindings.push(Effect::new(move || {
            let value = theme_value.get();
            if let Some(theme) = weak.upgrade() {
                theme.apply_config(&value);
                load_font_for(&theme, &fonts_for_effect);
            }
        }));
        if let Some(dir) = matugen_path.parent() {
            let _ = std::fs::create_dir_all(dir);
            let watch = Watch::file(handle, &matugen_path, move |_| load_matugen())?;
            theme.sources.borrow_mut().watch = Some(watch);
        }
        Ok(ThemeService { theme, bindings })
    }

    /// Number of bindings the service holds.
    pub fn bindings(&self) -> usize {
        self.bindings.len()
    }
}

fn load_font_for(theme: &Theme, fonts: &Fonts) {
    let family = theme.font_family.get_untracked();
    let path = {
        let sources = theme.sources.borrow();
        if sources.loaded_fonts.iter().any(|f| f.eq_ignore_ascii_case(&family)) {
            return;
        }
        match sources.catalog.find(&family) {
            Some(entry) => entry.path.clone(),
            None => return,
        }
    };
    fonts.load_file(&path);
    theme.sources.borrow_mut().loaded_fonts.push(family);
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn color(h: u32) -> Color {
        Color::hex(h)
    }

    #[test]
    fn reduced_motion_follows_the_setting_or_the_forced_switch() {
        let theme = Theme::new();
        assert!(!theme.reduced_motion.get_untracked());
        theme.reduced_motion_setting.set(true);
        assert!(theme.reduced_motion.get_untracked());
        assert!(acanthus_anim::reduced_motion());
        theme.reduced_motion_setting.set(false);
        assert!(!theme.reduced_motion.get_untracked());
        theme.reduced_motion_forced.set(true);
        assert!(theme.reduced_motion.get_untracked());
        drop(theme);
    }

    #[test]
    fn a_reduced_theme_makes_followed_signals_jump() {
        let theme = Theme::new();
        let src = Signal::new(0.0f32);
        let anim = acanthus_anim::Animated::new(&src, acanthus_anim::Motion::tween(100, acanthus_anim::Easing::Linear));
        theme.reduced_motion_forced.set(true);
        src.set(1.0);
        assert_eq!(anim.get(), 1.0);
        theme.reduced_motion_forced.set(false);
        assert!(!acanthus_anim::reduced_motion());
    }

    #[test]
    fn default_tokens_match_the_widget_durations() {
        let tokens = Theme::new().tokens;
        assert_eq!((tokens.motion.fast, tokens.motion.base, tokens.motion.slow), (150, 250, 400));
        assert!(tokens.spacing.xs < tokens.spacing.sm && tokens.spacing.lg < tokens.spacing.xl);
        assert!(tokens.radii.sm < tokens.radii.md && tokens.radii.lg < tokens.radii.pill);
    }

    #[test]
    fn border_radius_is_softly_clamped_above_24() {
        assert_eq!(clamp_border_radius(10), 10);
        assert_eq!(clamp_border_radius(24), 24);
        assert_eq!(clamp_border_radius(100), 24 + (76f64.powf(0.55)).floor() as i64);
        assert!(clamp_border_radius(1000) < 100);
    }

    #[test]
    fn matugen_detection_matches_the_original_rules() {
        assert!(is_matugen_theme(&json!({"matugen": true})));
        assert!(!is_matugen_theme(&json!({"matugen": false, "activePreset": "Matugen"})));
        assert!(is_matugen_theme(&json!({"activePreset": "Matugen"})));
        assert!(is_matugen_theme(&json!({})));
        assert!(!is_matugen_theme(&json!({"activePreset": "Dracula"})));
        assert!(is_matugen_theme(&json!({"name": "matugen", "activePreset": "x"})));
    }

    #[test]
    fn colors_accept_nested_hex_forms() {
        let theme = Theme::new();
        let changed = theme.apply_colors(&json!({"colors": {
            "base": "#282a36",
            "mantle": {"hex": "#1f202a"},
            "crust": {"color": "#191a21"},
            "text": {"default": {"hex": "#f8f8f2"}},
            "red": "not a color",
            "unknown_role": "#ffffff"
        }}));
        assert_eq!(changed, 4);
        assert_eq!(theme.palette.base.get_untracked(), color(0x282a36));
        assert_eq!(theme.palette.mantle.get_untracked(), color(0x1f202a));
        assert_eq!(theme.palette.crust.get_untracked(), color(0x191a21));
        assert_eq!(theme.text.get_untracked(), color(0xf8f8f2));
        assert_eq!(theme.palette.red.get_untracked(), color(0xf38ba8));
        assert_eq!(theme.apply_colors(&json!({"base": "#282a36"})), 0);
    }

    #[test]
    fn blur_background_keeps_the_configured_opacity_and_text_colours() {
        let theme = Theme::new();
        theme.apply_config(&json!({"activePreset": "Dracula", "matugen": false, "blurBackground": true, "blurStyle": "liquid", "elementOpacity": 50}));
        assert!(theme.glass_active.get_untracked());
        assert_eq!(theme.element_opacity.get_untracked(), 0.5);
        assert_ne!(theme.text.get_untracked(), Color::WHITE);
        assert_eq!(theme.faded(Color::BLACK).a, 0.5);
    }

    #[test]
    fn standalone_ignores_transparency_and_blur() {
        let theme = Theme::new();
        theme.standalone.set(true);
        theme.apply_config(&json!({"blurBackground": true, "elementOpacity": 30}));
        assert!(!theme.blur_background.get_untracked());
        assert_eq!(theme.element_opacity.get_untracked(), 1.0);
    }

    #[test]
    fn fonts_radius_and_primary_follow_config() {
        let theme = Theme::new();
        theme.apply_config(
            &json!({"fontFamily": "JetBrains Mono", "borderRadius": 40, "matugen": false, "activePreset": "Dracula"}),
        );
        assert_eq!(theme.font_family.get_untracked(), "JetBrains Mono");
        assert_eq!(theme.border_radius.get_untracked(), 40);
        assert_eq!(theme.clamped_border_radius.get_untracked(), clamp_border_radius(40));
        assert_eq!(theme.primary.get_untracked(), theme.palette.blue.get_untracked());
        assert_eq!(theme.icon_font.get_untracked(), "JetBrains Mono");
    }

    #[test]
    fn presets_resolve_from_the_bundled_themes() {
        let paths = crate::paths::test_paths();
        let names = preset_names(&paths.themes_dir());
        assert!(names.len() >= 3, "{}", names.len());
        let theme = Theme::new();
        theme.sources.borrow_mut().themes_dir = paths.themes_dir();
        theme.apply_config(&json!({"activePreset": "Dracula", "matugen": false}));
        assert_eq!(theme.palette.base.get_untracked(), color(0x282a36));
        assert_eq!(theme.palette.green.get_untracked(), color(0x50fa7b));
    }

    #[test]
    fn every_bundled_preset_parses_into_all_palette_roles() {
        let paths = crate::paths::test_paths();
        let mut checked = 0;
        for name in preset_names(&paths.themes_dir()) {
            let Some(colors) = load_preset(&paths.themes_dir(), &name) else { continue };
            if colors.get("isMatugen").is_some() {
                continue;
            }
            checked += 1;
            let theme = Theme::new();
            theme.apply_colors(&colors);
            for role in Palette::NAMES {
                let value = colors.get(*role).and_then(resolve_hex);
                assert!(value.as_deref().and_then(parse_color).is_some(), "{name}.{role}: {value:?}");
            }
        }
        assert!(checked >= 3, "{checked}");
    }

    #[test]
    fn matugen_colors_apply_only_for_matugen_themes() {
        let theme = Theme::new();
        theme.set_matugen_colors(Some(json!({"colors": {"base": "#010203"}})));
        theme.apply_config(&json!({"matugen": true}));
        assert_eq!(theme.palette.base.get_untracked(), color(0x010203));
        theme.apply_config(&json!({"matugen": false, "colors": {"base": "#0a0b0c"}}));
        assert_eq!(theme.palette.base.get_untracked(), color(0x0a0b0c));
    }
}
