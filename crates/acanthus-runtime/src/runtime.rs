use std::rc::Rc;

use acanthus_core::{Effect, Signal, untrack};
use acanthus_io::{IpcRegistry, IpcServer};
use acanthus_scene::Fonts;
use calloop::LoopHandle;
use serde_json::{Value, json};

use crate::{
    Clock, Config, DEFAULT_FULL_DATE, DEFAULT_TIME_FORMAT, FontCatalog, I18n, Matugen, NativeRunner, Paths, Scaler,
    ThemeService, is_matugen_theme, parse_scalar, system_language,
};

/// The runtime services of one application, started together.
pub struct Runtime {
    /// Directories.
    pub paths: Paths,
    /// Settings.
    pub config: Config,
    /// Translations.
    pub i18n: I18n,
    /// Clock.
    pub clock: Clock,
    /// Theme.
    pub theme: ThemeService,
    /// Interface scale per screen.
    pub scaler: Rc<Scaler>,
    /// Theme generation.
    pub matugen: Rc<Matugen>,
    /// Commands served over IPC.
    pub ipc: IpcRegistry,
    /// Fonts.
    pub fonts: Fonts,
    ipc_server: Option<IpcServer>,
    effects: Vec<Effect>,
}

fn string_arg(args: &[Value], index: usize) -> Option<String> {
    args.get(index).map(|v| v.as_str().map_or_else(|| v.to_string(), str::to_owned))
}

impl Runtime {
    /// Starts every service in `handle`'s event loop; `defaults` are the default settings and `serve_ipc` starts the IPC server.
    pub fn start<D: 'static>(
        handle: &LoopHandle<'static, D>,
        fonts: Fonts,
        paths: Paths,
        defaults: Value,
        serve_ipc: bool,
    ) -> Result<Runtime, acanthus_io::IoError> {
        let config = Config::load_with_defaults(&paths.settings, defaults);
        config.attach(handle)?;

        let language = config.value("general.language");
        let initial_language = language
            .get_untracked()
            .as_str()
            .filter(|l| !l.is_empty())
            .map(str::to_owned)
            .unwrap_or_else(|| system_language(|k| std::env::var(k).ok()));
        let i18n = I18n::new(&paths.languages_dir(), &initial_language);
        i18n.attach(handle)?;

        let mut effects = Vec::new();
        {
            let (i18n, language) = (i18n.clone(), language.clone());
            effects.push(Effect::new(move || {
                let wanted = language
                    .get()
                    .as_str()
                    .filter(|l| !l.is_empty())
                    .map(str::to_owned)
                    .unwrap_or_else(|| system_language(|k| std::env::var(k).ok()));
                untrack(|| i18n.set_language(&wanted));
            }));
        }

        let time_format = Signal::new(DEFAULT_TIME_FORMAT.to_owned());
        {
            let (target, config) = (time_format.clone(), config.clone());
            effects.push(Effect::new(move || {
                config.revision().get();
                let pick = |path: &str| config.user_value(path).and_then(|v| v.as_str().map(str::to_owned));
                let next = pick("bar.time.format")
                    .or_else(|| pick("general.time_format"))
                    .unwrap_or_else(|| DEFAULT_TIME_FORMAT.to_owned());
                untrack(|| target.set(next));
            }));
        }
        let full_date = Signal::new(DEFAULT_FULL_DATE.to_owned());
        {
            let (target, i18n) = (full_date.clone(), i18n.clone());
            effects.push(Effect::new(move || {
                let next = i18n.tr("datetime.full_date", DEFAULT_FULL_DATE);
                untrack(|| target.set(next));
            }));
        }
        let clock = Clock::new(crate::local_now, time_format, i18n.language(), full_date);
        clock.attach(handle)?;

        let theme = ThemeService::start(&config, &paths, &fonts, handle)?;
        let scaler = Scaler::new(&config);
        let matugen = Matugen::new(&config, &paths, NativeRunner::new(handle, paths.home.clone())?);

        {
            let (theme_value, matugen, config) = (config.value("theme"), matugen.clone(), config.clone());
            effects.push(Effect::new(move || {
                let theme = theme_value.get();
                let mode = theme.get("mode").and_then(Value::as_str).map(str::to_owned);
                untrack(|| {
                    if is_matugen_theme(&theme) {
                        let wallpaper = theme
                            .get("wallpaper")
                            .and_then(Value::as_str)
                            .map(str::to_owned)
                            .or_else(|| config.current("wallpaper").as_str().map(str::to_owned))
                            .unwrap_or_default();
                        if !wallpaper.is_empty() {
                            let scheme = theme.get("schemeType").and_then(Value::as_str);
                            matugen.generate(&wallpaper, mode.as_deref(), scheme);
                        }
                    } else if let Some(colors) = theme
                        .get("colors")
                        .or_else(|| theme.get("palette"))
                        .or_else(|| theme.get("base").map(|_| &theme))
                        .filter(|v| v.is_object())
                    {
                        matugen.generate_from_static(colors, mode.as_deref());
                    }
                });
            }));
        }

        let ipc = IpcRegistry::new();
        Runtime::register_ipc(&ipc, &config, &theme, &matugen, &fonts, &paths);
        let ipc_server = if serve_ipc { Some(IpcServer::bind(handle, &paths.ipc_socket, ipc.clone())?) } else { None };

        Ok(Runtime {
            paths,
            config,
            i18n,
            clock,
            theme,
            scaler: Rc::new(scaler),
            matugen,
            ipc,
            fonts,
            ipc_server,
            effects,
        })
    }

    fn register_ipc(
        ipc: &IpcRegistry,
        config: &Config,
        theme: &ThemeService,
        matugen: &Rc<Matugen>,
        fonts: &Fonts,
        paths: &Paths,
    ) {
        let (cfg, th) = (config.clone(), theme.theme.clone());
        ipc.register("theme", "reloadColors", move |_| {
            th.apply_config(&cfg.current("theme"));
            Ok(Value::Null)
        });

        let (th, dirs) = (theme.theme.clone(), [paths.assets.join("fonts"), paths.state_dir.join("fonts")]);
        let _ = fonts;
        ipc.register("fonts", "rescan", move |_| {
            let catalog = FontCatalog::scan(&dirs);
            let count = catalog.len();
            th.set_catalog(catalog);
            Ok(json!(count))
        });

        let m = matugen.clone();
        ipc.register("matugen", "generate", move |args| {
            let image = string_arg(args, 0).unwrap_or_default();
            Ok(json!(m.generate(&image, string_arg(args, 1).as_deref(), string_arg(args, 2).as_deref())))
        });
        let m = matugen.clone();
        ipc.register("matugen", "generateImage", move |args| {
            Ok(json!(m.generate(&string_arg(args, 0).unwrap_or_default(), None, None)))
        });
        let m = matugen.clone();
        ipc.register("matugen", "generateFromStatic", move |args| {
            let parsed: Value =
                serde_json::from_str(&string_arg(args, 0).unwrap_or_default()).map_err(|e| e.to_string())?;
            Ok(json!(m.generate_from_static(&parsed, string_arg(args, 1).as_deref())))
        });

        let c = config.clone();
        ipc.register("config", "get", move |args| {
            let path = string_arg(args, 0).ok_or("config.get needs a key")?;
            Ok(c.current(&path))
        });
        let c = config.clone();
        ipc.register("config", "set", move |args| {
            let path = string_arg(args, 0).ok_or("config.set needs a key")?;
            let raw = args.get(1).ok_or("config.set needs a value")?;
            let value = match raw {
                Value::String(s) => parse_scalar(s),
                other => other.clone(),
            };
            c.set(&path, value);
            Ok(json!(true))
        });
        let c = config.clone();
        ipc.register("config", "unset", move |args| {
            c.unset(&string_arg(args, 0).ok_or("config.unset needs a key")?);
            Ok(json!(true))
        });
        let c = config.clone();
        ipc.register("config", "dump", move |_| Ok(json!(c.dump())));
        let c = config.clone();
        ipc.register("config", "path", move |_| Ok(json!(c.path().to_string_lossy())));
    }

    /// Socket path of the IPC server, if one runs.
    pub fn ipc_path(&self) -> Option<&std::path::Path> {
        self.ipc_server.as_ref().map(IpcServer::path)
    }

    /// Writes pending settings immediately.
    pub fn flush(&self) {
        self.config.flush();
    }

    /// Number of reactive effects the runtime holds.
    pub fn effects(&self) -> usize {
        self.effects.len()
    }
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use acanthus_io::IpcClient;
    use calloop::EventLoop;

    use super::*;

    struct Env {
        _dir: tempfile::TempDir,
        paths: Paths,
        event_loop: EventLoop<'static, ()>,
    }

    fn env(settings: &str) -> Env {
        let dir = tempfile::tempdir().unwrap();
        let mut paths = crate::paths::test_paths();
        paths.settings = dir.path().join("config/settings.json");
        paths.state_dir = dir.path().join("state");
        paths.cache_dir = dir.path().join("cache");
        paths.runtime_dir = dir.path().join("run");
        paths.ipc_socket = dir.path().join("run/ipc.sock");
        std::fs::create_dir_all(paths.settings.parent().unwrap()).unwrap();
        std::fs::write(&paths.settings, settings).unwrap();
        Env { _dir: dir, paths, event_loop: EventLoop::try_new().unwrap() }
    }

    fn demo_defaults() -> Value {
        serde_json::from_str(include_str!("fixtures/demo_settings.json")).unwrap()
    }

    fn pump(event_loop: &mut EventLoop<'static, ()>, ms: u64) {
        let end = Instant::now() + Duration::from_millis(ms);
        while Instant::now() < end {
            event_loop.dispatch(Duration::from_millis(5), &mut ()).unwrap();
        }
    }

    #[test]
    fn start_wires_language_time_and_theme_from_settings() {
        let mut env = env(r##"{"general": {"language": "de"}, "bar": {"time": {"format": "h:mm AP"}},
                "theme": {"matugen": false, "activePreset": "Dracula", "borderRadius": 30}}"##);
        let runtime =
            Runtime::start(&env.event_loop.handle(), Fonts::new(), env.paths.clone(), demo_defaults(), false).unwrap();
        assert_eq!(runtime.i18n.language().get_untracked(), "de");
        assert!(runtime.clock.is_12_hour().get_untracked());
        assert_eq!(runtime.theme.theme.palette.base.get_untracked(), acanthus_core::Color::hex(0x282a36));
        assert_eq!(runtime.theme.theme.border_radius.get_untracked(), 30);
        assert_ne!(runtime.i18n.t("fixture.hello"), "fixture.hello");

        runtime.config.set("general.language", json!("fr"));
        assert_eq!(runtime.i18n.language().get_untracked(), "fr");
        runtime.config.set("bar.time.format", json!("HH:mm"));
        assert!(!runtime.clock.is_12_hour().get_untracked());
        runtime.config.set("theme.activePreset", json!("Nord"));
        pump(&mut env.event_loop, 20);
        assert_ne!(runtime.theme.theme.palette.base.get_untracked(), acanthus_core::Color::hex(0x282a36));
    }

    #[test]
    fn general_time_format_is_the_fallback_when_bar_has_none() {
        let env =
            env(r#"{"general": {"time_format": "hh:mm a"}, "theme": {"matugen": false, "activePreset": "Nord"}}"#);
        let runtime =
            Runtime::start(&env.event_loop.handle(), Fonts::new(), env.paths.clone(), demo_defaults(), false).unwrap();
        assert!(runtime.clock.is_12_hour().get_untracked());
        assert!(runtime.effects() > 0);
    }

    #[test]
    fn ipc_serves_config_and_theme_targets_with_original_names() {
        let mut env = env(r#"{"bar": {"height": 52}, "theme": {"matugen": false, "activePreset": "Nord"}}"#);
        let runtime =
            Runtime::start(&env.event_loop.handle(), Fonts::new(), env.paths.clone(), demo_defaults(), true).unwrap();
        let socket = runtime.ipc_path().unwrap().to_path_buf();
        let methods = runtime.ipc.methods();
        for expected in [
            ("theme", "reloadColors"),
            ("fonts", "rescan"),
            ("matugen", "generate"),
            ("matugen", "generateImage"),
            ("matugen", "generateFromStatic"),
        ] {
            assert!(methods.contains(&(expected.0.to_owned(), expected.1.to_owned())), "{expected:?}");
        }

        let call = |target: &'static str, method: &'static str, args: Vec<Value>| {
            let socket = socket.clone();
            std::thread::spawn(move || IpcClient::call(&socket, target, method, &args))
        };
        let mut await_job = |job: std::thread::JoinHandle<Result<Value, acanthus_io::IoError>>| {
            let deadline = Instant::now() + Duration::from_secs(10);
            while !job.is_finished() && Instant::now() < deadline {
                env.event_loop.dispatch(Duration::from_millis(5), &mut ()).unwrap();
            }
            job.join().unwrap().unwrap()
        };
        let height = await_job(call("config", "get", vec![json!("bar.height")]));
        await_job(call("config", "set", vec![json!("bar.height"), json!("60")]));
        let fonts = await_job(call("fonts", "rescan", vec![]));
        assert_eq!(height, json!(52));
        assert!(fonts.as_u64().unwrap() >= 2);
        assert_eq!(runtime.config.get_now("bar.height", 0), 60);
    }

    #[test]
    fn invalid_ipc_arguments_report_errors_without_panicking() {
        let env = env("{}");
        let event_loop: EventLoop<'static, ()> = EventLoop::try_new().unwrap();
        let runtime =
            Runtime::start(&event_loop.handle(), Fonts::new(), env.paths.clone(), demo_defaults(), false).unwrap();
        assert!(runtime.ipc.call("config", "get", &[]).is_err());
        assert!(runtime.ipc.call("matugen", "generateFromStatic", &[json!("not json")]).is_err());
        assert!(runtime.ipc.call("matugen", "generate", &[json!("")]).unwrap() == json!(false));
    }
}
