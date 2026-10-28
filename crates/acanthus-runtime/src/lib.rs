//! Runtime services shared by every application: settings, translations, clock, theme, scaling, paths, colour generation and IPC wiring.

#![deny(missing_docs)]
mod clock;
mod colorgen;
mod config;
mod font_catalog;
mod i18n;
mod json;
pub mod lenient;
mod matugen;
mod paths;
mod qt_format;
mod runtime;
mod scaler;
mod section;
mod theme;

pub use acanthus_io::AppId;
pub use clock::{Clock, DEFAULT_FULL_DATE, DEFAULT_TIME_FORMAT, TimeFormat, local_now};
pub use colorgen::{
    ColorGenError, ColorTable, ImageRequest, Mode as ColorMode, TemplateSpec, apply_templates, parse_template_config,
    render_template, signal_processes_named, table_from_image, table_from_source,
};
pub use config::{Config, ConfigError, WRITE_DEBOUNCE};
pub use font_catalog::{FontCatalog, FontEntry};
pub use i18n::{I18n, is_rtl, system_language};
pub use json::{get_path, parse_scalar, remove_path, set_path};
pub use matugen::{
    Matugen, MatugenHooks, MatugenJob, MatugenRequest, MatugenRunner, NativeRunner, execute_job, to_md3,
};
pub use paths::Paths;
pub use qt_format::{format_qt, locale_for};
pub use runtime::Runtime;
pub use scaler::Scaler;
pub use section::Section;
pub use theme::{Palette, Theme, ThemeService, clamp_border_radius, is_matugen_theme, load_preset, preset_names};
