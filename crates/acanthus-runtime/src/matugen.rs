use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use acanthus_core::Signal;
use acanthus_io::atomic_write;
use serde_json::{Map, Value, json};

use acanthus_color::generate::SchemeType;

use crate::{
    ColorGenError, ColorMode, ColorTable, Config, ImageRequest, Paths, SettingKeys, apply_templates, is_matugen_theme,
    parse_template_config, signal_processes_named, table_from_image,
};

pub const DEFAULT_MODE: &str = "dark";
pub const DEFAULT_SCHEME: &str = "scheme-tonal-spot";

/// A theme generation request.
#[derive(Debug, Clone, PartialEq)]
pub enum MatugenRequest {
    /// Generation from an image.
    Image {
        /// Image file.
        path: String,
        /// Dark, light or smart.
        mode: String,
        /// Scheme variant name.
        scheme: String,
    },
    /// Generation from a ready colour table.
    Static {
        /// Colour table as JSON.
        colors: Value,
        /// Dark or light.
        mode: String,
    },
}

/// A theme generation job handed to a [`MatugenRunner`].
#[derive(Debug, Clone, PartialEq)]
pub enum MatugenJob {
    /// Generation from an image.
    Image {
        /// Image file.
        path: PathBuf,
        /// Dark, light or smart.
        mode: String,
        /// Scheme variant name.
        scheme: String,
        /// Template configuration to render.
        config: PathBuf,
    },
    /// Generation from a ready colour table.
    Static {
        /// Colour table as JSON.
        colors: Value,
        /// Template configuration to render.
        config: PathBuf,
    },
}

/// Executes theme generation jobs, typically off the main thread.
pub trait MatugenRunner {
    /// Runs `job` and calls `done` with the outcome.
    fn run(&self, job: MatugenJob, done: Box<dyn FnOnce(bool)>);
}

/// Reacts to finished theme generation.
pub trait MatugenHooks {
    /// Called after `request` succeeded.
    fn after_success(&self, request: &MatugenRequest);
}

#[derive(Default)]
struct State {
    current: Option<MatugenRequest>,
    pending: Option<MatugenRequest>,
    last_success: Option<MatugenRequest>,
    last_wallpaper: String,
}

/// Theme generation scheduler: runs one job at a time, keeps the latest pending request and skips repeats of the last success.
pub struct Matugen {
    state: RefCell<State>,
    runner: Rc<dyn MatugenRunner>,
    hooks: Rc<dyn MatugenHooks>,
    config: Config,
    theme_key: String,
    base_dir: PathBuf,
    state_dir: PathBuf,
    running: Signal<bool>,
    finished: Signal<u64>,
    last_result: Signal<Option<bool>>,
}

struct FileHooks {
    state_dir: PathBuf,
}

impl MatugenHooks for FileHooks {
    fn after_success(&self, request: &MatugenRequest) {
        if matches!(request, MatugenRequest::Image { .. }) {
            let _ = std::fs::copy(self.state_dir.join("colors.json"), self.state_dir.join("matugen_colors.json"));
        }
        if signal_processes_named(&[".kitty-wrapped"], rustix::process::Signal::USR1) == 0 {
            signal_processes_named(&["kitty"], rustix::process::Signal::USR1);
        }
    }
}

pub fn clean_image_path(path: &str) -> &str {
    path.strip_prefix("file://").unwrap_or(path)
}

fn luminance(hex: &str) -> f64 {
    let c = hex.trim_start_matches('#');
    let c: String = if c.len() == 3 { c.chars().flat_map(|ch| [ch, ch]).collect() } else { c.to_owned() };
    let channel = |range: std::ops::Range<usize>| {
        c.get(range).and_then(|s| u8::from_str_radix(s, 16).ok()).map_or(0.0, |v| f64::from(v) / 255.0)
    };
    0.2126 * channel(0..2) + 0.7152 * channel(2..4) + 0.0722 * channel(4..6)
}

/// Converts a Catppuccin-style palette to Material 3 role names.
pub fn to_md3(colors: &Value) -> Value {
    let get = |name: &str, fallback: &str| -> String {
        colors.get(name).and_then(Value::as_str).unwrap_or(fallback).to_owned()
    };
    let crust = get("crust", "#11111b");
    let text = get("text", "#cdd6f4");
    let on_color = |hex: &str| if luminance(hex) > 0.5 { crust.clone() } else { text.clone() };

    let mut md3: Vec<(String, String)> = [
        ("surface_container_lowest", "base"),
        ("surface_container_low", "mantle"),
        ("surface", "crust"),
        ("surface_container", "surface0"),
        ("surface_container_high", "surface1"),
        ("surface_container_highest", "surface2"),
        ("surface_variant", "surface1"),
        ("on_surface", "text"),
        ("on_surface_variant", "subtext0"),
        ("outline", "subtext1"),
        ("inverse_surface", "overlay0"),
        ("primary", "blue"),
        ("primary_container", "sapphire"),
        ("secondary", "green"),
        ("secondary_container", "yellow"),
        ("tertiary", "peach"),
        ("tertiary_container", "pink"),
        ("error", "red"),
        ("error_container", "maroon"),
        ("primary_fixed", "sapphire"),
        ("secondary_fixed", "yellow"),
        ("tertiary_fixed", "pink"),
        ("inverse_primary", "mauve"),
    ]
    .iter()
    .map(|(role, source)| ((*role).to_owned(), get(source, "#000000")))
    .collect();

    for role in ["primary", "secondary", "tertiary", "error"] {
        let base = md3.iter().find(|(r, _)| r == role).map(|(_, v)| v.clone()).unwrap_or_default();
        let container = md3.iter().find(|(r, _)| *r == format!("{role}_container")).map(|(_, v)| v.clone());
        md3.push((format!("on_{role}"), on_color(&base)));
        md3.push((format!("on_{role}_container"), on_color(&container.unwrap_or_default())));
    }

    let mut out = Map::new();
    for (role, hex) in md3 {
        out.insert(
            role,
            json!({
                "default": {"hex": hex, "color": hex},
                "dark": {"hex": hex, "color": hex},
                "light": {"hex": hex, "color": hex},
            }),
        );
    }
    json!({"colors": Value::Object(out)})
}

impl Matugen {
    /// Scheduler using the theme section named by `keys`, the paths and `runner`.
    pub fn new(config: &Config, keys: &SettingKeys, paths: &Paths, runner: Rc<dyn MatugenRunner>) -> Rc<Matugen> {
        let hooks = Rc::new(FileHooks { state_dir: paths.state_dir.clone() });
        Matugen::with_hooks(config, keys, paths, runner, hooks)
    }

    /// Like [`Matugen::new`] with custom `hooks`.
    pub fn with_hooks(
        config: &Config,
        keys: &SettingKeys,
        paths: &Paths,
        runner: Rc<dyn MatugenRunner>,
        hooks: Rc<dyn MatugenHooks>,
    ) -> Rc<Matugen> {
        Rc::new(Matugen {
            state: RefCell::default(),
            runner,
            hooks,
            config: config.clone(),
            theme_key: keys.theme.clone(),
            base_dir: paths.assets.join("matugen"),
            state_dir: paths.state_dir.clone(),
            running: Signal::new(false),
            finished: Signal::new(0),
            last_result: Signal::new(None),
        })
    }

    /// Signal that is true while a job runs.
    pub fn running(&self) -> Signal<bool> {
        self.running.clone()
    }

    /// Counter of finished jobs.
    pub fn finished(&self) -> Signal<u64> {
        self.finished.clone()
    }

    /// Outcome of the last finished job.
    pub fn last_result(&self) -> Signal<Option<bool>> {
        self.last_result.clone()
    }

    /// Image of the last request.
    pub fn last_wallpaper(&self) -> String {
        self.state.borrow().last_wallpaper.clone()
    }

    fn theme_value(&self) -> Value {
        self.config.current(&self.theme_key)
    }

    /// Whether the configured theme asks for generated colours.
    pub fn is_matugen_theme(&self) -> bool {
        is_matugen_theme(&self.theme_value())
    }

    /// Requests colours from `image`; `mode` and `scheme` default to the theme settings. False when the request was ignored.
    pub fn generate(self: &Rc<Self>, image: &str, mode: Option<&str>, scheme: Option<&str>) -> bool {
        if image.trim().is_empty() || !self.is_matugen_theme() {
            return false;
        }
        let theme = self.theme_value();
        let pick = |explicit: Option<&str>, key: &str, default: &str| {
            explicit.filter(|s| !s.is_empty()).map(str::to_owned).unwrap_or_else(|| {
                theme.get(key).and_then(Value::as_str).filter(|s| !s.is_empty()).unwrap_or(default).to_owned()
            })
        };
        self.submit(MatugenRequest::Image {
            path: clean_image_path(image).to_owned(),
            mode: pick(mode, "mode", DEFAULT_MODE),
            scheme: pick(scheme, "schemeType", DEFAULT_SCHEME),
        })
    }

    /// Requests a theme from ready `colors`; false when the request was ignored.
    pub fn generate_from_static(self: &Rc<Self>, colors: &Value, mode: Option<&str>) -> bool {
        if colors.is_null() {
            return false;
        }
        let mode = mode
            .filter(|m| !m.is_empty())
            .map(str::to_owned)
            .or_else(|| self.theme_value().get("mode").and_then(Value::as_str).map(str::to_owned))
            .unwrap_or_else(|| DEFAULT_MODE.to_owned());
        self.submit(MatugenRequest::Static { colors: colors.clone(), mode })
    }

    fn submit(self: &Rc<Self>, request: MatugenRequest) -> bool {
        {
            let mut state = self.state.borrow_mut();
            let running = self.running.get_untracked();
            if !running && state.last_success.as_ref() == Some(&request) {
                return true;
            }
            if running {
                if state.current.as_ref() == Some(&request) || state.pending.as_ref() == Some(&request) {
                    return true;
                }
                state.pending = Some(request);
                return false;
            }
        }
        self.start(request);
        true
    }

    fn start(self: &Rc<Self>, request: MatugenRequest) {
        let job = match &request {
            MatugenRequest::Image { path, mode, scheme } => {
                self.state.borrow_mut().last_wallpaper = path.clone();
                MatugenJob::Image {
                    path: PathBuf::from(path),
                    mode: mode.clone(),
                    scheme: scheme.clone(),
                    config: self.base_dir.join("config.toml"),
                }
            }
            MatugenRequest::Static { colors, .. } => {
                if let Err(e) = atomic_write(&self.state_dir.join("colors.json"), colors.to_string().as_bytes()) {
                    tracing::error!("cannot stage matugen colors: {e}");
                    self.state.borrow_mut().current = Some(request);
                    self.finish(false);
                    return;
                }
                MatugenJob::Static { colors: to_md3(colors), config: self.base_dir.join("config-static.toml") }
            }
        };
        self.state.borrow_mut().current = Some(request);
        self.running.set(true);
        let this = Rc::clone(self);
        self.runner.run(job, Box::new(move |success| this.finish(success)));
    }

    fn finish(self: &Rc<Self>, success: bool) {
        let (finished, pending) = {
            let mut state = self.state.borrow_mut();
            let finished = state.current.take();
            if success {
                state.last_success = finished.clone();
            }
            (finished, state.pending.take())
        };
        if success && let Some(request) = &finished {
            self.hooks.after_success(request);
        }
        acanthus_core::batch(|| {
            self.running.set(false);
            self.last_result.set(Some(success));
            self.finished.update(|n| *n += 1);
        });
        if let Some(next) = pending {
            let redundant = success && self.state.borrow().last_success.as_ref() == Some(&next);
            if !redundant {
                self.start(next);
            }
        }
    }

    /// Directory where generated files are kept.
    pub fn state_dir(&self) -> &Path {
        &self.state_dir
    }
}

/// Runs `job` in the calling thread.
pub fn execute_job(job: &MatugenJob, home: &Path) -> Result<(), ColorGenError> {
    match job {
        MatugenJob::Image { path, mode, scheme, config } => {
            let mode = ColorMode::parse(mode).unwrap_or(ColorMode::Dark);
            let scheme = SchemeType::parse(scheme).unwrap_or_default();
            let table =
                table_from_image(&ImageRequest { path: path.clone(), mode, scheme, contrast: None, source_index: 0 })?;
            render_config(config, &table, home)
        }
        MatugenJob::Static { colors, config } => {
            let table = ColorTable::from_json(colors, true)?;
            render_config(config, &table, home)
        }
    }
}

fn render_config(config: &Path, table: &ColorTable, home: &Path) -> Result<(), ColorGenError> {
    let text = std::fs::read_to_string(config)?;
    let specs = parse_template_config(&text, config.parent().unwrap_or(Path::new(".")), home)?;
    apply_templates(&specs, table).map(|_| ())
}

type Pending = Rc<RefCell<HashMap<u64, Box<dyn FnOnce(bool)>>>>;

/// Runner that generates themes on a worker thread.
pub struct NativeRunner {
    sender: calloop::channel::Sender<(u64, bool)>,
    pending: Pending,
    next: Cell<u64>,
    home: PathBuf,
    remove: Option<Box<dyn FnOnce()>>,
}

impl NativeRunner {
    /// Runner that reports to `handle`'s event loop; `home` expands `~` in template paths.
    pub fn new<D: 'static>(
        handle: &calloop::LoopHandle<'static, D>,
        home: PathBuf,
    ) -> Result<Rc<NativeRunner>, acanthus_io::IoError> {
        let (sender, channel) = calloop::channel::channel::<(u64, bool)>();
        let pending: Pending = Rc::default();
        let waiting = pending.clone();
        let token = handle.insert_source(channel, move |event, _, _| {
            if let calloop::channel::Event::Msg((id, ok)) = event {
                let done = waiting.borrow_mut().remove(&id);
                if let Some(done) = done {
                    done(ok);
                }
            }
        })?;
        let weak = acanthus_io::WeakHandle::new(handle);
        Ok(Rc::new(NativeRunner {
            sender,
            pending,
            next: Cell::new(1),
            home,
            remove: Some(Box::new(move || weak.remove(token))),
        }))
    }
}

impl Drop for NativeRunner {
    fn drop(&mut self) {
        if let Some(remove) = self.remove.take() {
            remove();
        }
    }
}

impl MatugenRunner for NativeRunner {
    fn run(&self, job: MatugenJob, done: Box<dyn FnOnce(bool)>) {
        let id = self.next.get();
        self.next.set(id + 1);
        self.pending.borrow_mut().insert(id, done);
        let (sender, home) = (self.sender.clone(), self.home.clone());
        std::thread::spawn(move || {
            let result = execute_job(&job, &home);
            if let Err(e) = &result {
                tracing::error!("colour generation failed: {e}");
            }
            let _ = sender.send((id, result.is_ok()));
        });
    }
}

#[cfg(test)]
mod tests {
    use std::cell::{Cell, RefCell};

    use super::*;

    type Completion = Box<dyn FnOnce(bool)>;

    #[derive(Default)]
    struct FakeRunner {
        jobs: RefCell<Vec<MatugenJob>>,
        completions: RefCell<Vec<Completion>>,
    }

    impl MatugenRunner for FakeRunner {
        fn run(&self, job: MatugenJob, done: Box<dyn FnOnce(bool)>) {
            self.jobs.borrow_mut().push(job);
            self.completions.borrow_mut().push(done);
        }
    }

    impl FakeRunner {
        fn complete(&self, success: bool) {
            let done = self.completions.borrow_mut().remove(0);
            done(success);
        }
    }

    #[derive(Default)]
    struct CountingHooks(Cell<u32>);

    impl MatugenHooks for CountingHooks {
        fn after_success(&self, _: &MatugenRequest) {
            self.0.set(self.0.get() + 1);
        }
    }

    struct Fixture {
        _dir: tempfile::TempDir,
        runner: Rc<FakeRunner>,
        hooks: Rc<CountingHooks>,
        matugen: Rc<Matugen>,
        config: Config,
    }

    fn fixture(theme: &str) -> Fixture {
        let dir = tempfile::tempdir().unwrap();
        let settings = dir.path().join("settings.json");
        std::fs::write(&settings, format!(r#"{{"theme": {theme}}}"#)).unwrap();
        let config = Config::load_demo(&settings);
        let mut paths = crate::paths::test_paths();
        paths.state_dir = dir.path().join("state");
        let runner = Rc::new(FakeRunner::default());
        let hooks = Rc::new(CountingHooks::default());
        let matugen = Matugen::with_hooks(&config, &SettingKeys::default(), &paths, runner.clone(), hooks.clone());
        Fixture { _dir: dir, runner, hooks, matugen, config }
    }

    fn image_path(job: &MatugenJob) -> String {
        match job {
            MatugenJob::Image { path, .. } => path.to_string_lossy().into_owned(),
            MatugenJob::Static { .. } => String::new(),
        }
    }

    #[test]
    fn image_request_becomes_a_typed_job_with_theme_defaults() {
        let f = fixture(r#"{"matugen": true, "mode": "light"}"#);
        assert!(f.matugen.generate("file:///w/a.png", None, None));
        let job = f.runner.jobs.borrow()[0].clone();
        let MatugenJob::Image { path, mode, scheme, config } = job else { panic!("image job expected") };
        assert_eq!(path, PathBuf::from("/w/a.png"));
        assert_eq!((mode.as_str(), scheme.as_str()), ("light", DEFAULT_SCHEME));
        assert!(config.ends_with("matugen/config.toml"));
        assert!(f.matugen.running().get_untracked());
        assert_eq!(f.matugen.last_wallpaper(), "/w/a.png");
    }

    #[test]
    fn rejects_blank_paths_and_non_matugen_themes() {
        let f = fixture(r#"{"matugen": false, "activePreset": "Dracula"}"#);
        assert!(!f.matugen.generate("/w/a.png", None, None));
        assert!(f.runner.jobs.borrow().is_empty());
        let g = fixture(r#"{"matugen": true}"#);
        assert!(!g.matugen.generate("  ", None, None));
    }

    #[test]
    fn duplicate_requests_are_coalesced_and_remembered() {
        let f = fixture(r#"{"matugen": true}"#);
        assert!(f.matugen.generate("/a.png", None, None));
        assert!(f.matugen.generate("/a.png", None, None));
        assert_eq!(f.runner.jobs.borrow().len(), 1);
        f.runner.complete(true);
        assert!(f.matugen.generate("/a.png", None, None));
        assert_eq!(f.runner.jobs.borrow().len(), 1);
        assert_eq!(f.hooks.0.get(), 1);
        assert_eq!(f.matugen.last_result().get_untracked(), Some(true));
    }

    #[test]
    fn newer_request_waits_and_only_the_latest_pending_survives() {
        let f = fixture(r#"{"matugen": true}"#);
        f.matugen.generate("/a.png", None, None);
        assert!(!f.matugen.generate("/b.png", None, None));
        assert!(!f.matugen.generate("/c.png", None, None));
        assert!(f.matugen.generate("/c.png", None, None));
        assert_eq!(f.runner.jobs.borrow().len(), 1);
        f.runner.complete(true);
        assert_eq!(f.runner.jobs.borrow().len(), 2);
        assert_eq!(image_path(&f.runner.jobs.borrow()[1]), "/c.png");
        f.runner.complete(true);
        assert!(!f.matugen.running().get_untracked());
    }

    #[test]
    fn failure_does_not_remember_the_request_so_it_can_retry() {
        let f = fixture(r#"{"matugen": true}"#);
        f.matugen.generate("/a.png", None, None);
        f.runner.complete(false);
        assert_eq!(f.hooks.0.get(), 0);
        assert_eq!(f.matugen.last_result().get_untracked(), Some(false));
        f.matugen.generate("/a.png", None, None);
        assert_eq!(f.runner.jobs.borrow().len(), 2);
    }

    #[test]
    fn alternating_requests_each_run_once_in_order() {
        let f = fixture(r#"{"matugen": true}"#);
        f.matugen.generate("/a.png", None, None);
        f.matugen.generate("/b.png", None, None);
        f.runner.complete(true);
        f.matugen.generate("/a.png", None, None);
        f.runner.complete(true);
        f.runner.complete(true);
        let order: Vec<String> = f.runner.jobs.borrow().iter().map(image_path).collect();
        assert_eq!(order, ["/a.png", "/b.png", "/a.png"]);
        assert!(!f.matugen.running().get_untracked());
        assert!(f.runner.completions.borrow().is_empty());
        assert_eq!(f.hooks.0.get(), 3);
    }

    #[test]
    fn static_request_stages_the_raw_colors_and_hands_md3_to_the_static_config() {
        let f = fixture(r#"{"matugen": false}"#);
        let colors = json!({"base": "#112233", "text": "#ffffff", "crust": "#000000", "blue": "#ffffff"});
        assert!(f.matugen.generate_from_static(&colors, None));
        let job = f.runner.jobs.borrow()[0].clone();
        let MatugenJob::Static { colors: md3, config } = job else { panic!("static job expected") };
        assert!(config.ends_with("matugen/config-static.toml"));
        assert_eq!(md3["colors"]["surface_container_lowest"]["default"]["hex"], "#112233");
        let staged: Value =
            serde_json::from_str(&std::fs::read_to_string(f.matugen.state_dir().join("colors.json")).unwrap()).unwrap();
        assert_eq!(staged, colors);
        let _ = &f.config;
    }

    #[test]
    fn md3_mapping_picks_readable_on_colors() {
        let md3 = to_md3(
            &json!({"crust": "#000011", "text": "#eeeeee", "blue": "#ffffff", "sapphire": "#101010", "red": "#000000"}),
        );
        let on = |role: &str| md3["colors"][role]["default"]["hex"].as_str().unwrap().to_owned();
        assert_eq!(on("on_primary"), "#000011");
        assert_eq!(on("on_primary_container"), "#eeeeee");
        assert_eq!(on("on_error"), "#eeeeee");
        assert_eq!(on("primary"), "#ffffff");
        assert_eq!(on("inverse_primary"), "#000000");
        assert_eq!(md3["colors"].as_object().unwrap().len(), 31);
    }
}
