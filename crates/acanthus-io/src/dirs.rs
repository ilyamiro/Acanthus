use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};

const KINDS: [&str; 4] = ["CACHE", "STATE", "RUN", "LOG"];
const SYSTEM_BASES: [&str; 4] = ["HOME", "XDG_RUNTIME_DIR", "XDG_DATA_HOME", "XDG_CACHE_HOME"];
const OVERRIDE_BASES: [&str; 5] = ["SETTINGS", "CACHE_DIR", "STATE_DIR", "RUN_DIR", "LOG_DIR"];

/// Identity of an application: the directory name used under the XDG bases and the prefix of its override variables.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppId {
    /// Directory and socket name, for example `myshell` gives `~/.config/myshell`.
    pub name: String,
    /// Prefix of the override variables, for example `MS` gives `MS_CACHE_DIR` and `MS_RUN_WEATHER`.
    pub env_prefix: String,
}

impl AppId {
    /// Creates an identity from the directory name and the override variable prefix.
    pub fn new(name: impl Into<String>, env_prefix: impl Into<String>) -> AppId {
        AppId { name: name.into(), env_prefix: env_prefix.into() }
    }

    /// Name of the override variable `<prefix>_<suffix>`.
    pub fn var(&self, suffix: &str) -> String {
        format!("{}_{suffix}", self.env_prefix)
    }
}

/// Every directory an application reads or writes, resolved from the environment with the standard per-app layout.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Dirs {
    /// The identity the directories were resolved for.
    pub app: AppId,
    /// Home directory.
    pub home: PathBuf,
    /// Configuration directory.
    pub config_dir: PathBuf,
    /// Path of the settings file.
    pub settings: PathBuf,
    /// Cache directory of the app.
    pub cache_dir: PathBuf,
    /// State directory of the app.
    pub state_dir: PathBuf,
    /// Runtime directory of the app.
    pub runtime_dir: PathBuf,
    /// Log directory.
    pub log_dir: PathBuf,
    /// Data directory under the XDG data home.
    pub data_dir: PathBuf,
    /// The XDG data home.
    pub xdg_data_home: PathBuf,
    /// The XDG cache home.
    pub xdg_cache_home: PathBuf,
    /// Path of the IPC socket.
    pub ipc_socket: PathBuf,
    /// Root of the bundled assets.
    pub assets: PathBuf,
    overrides: BTreeMap<String, PathBuf>,
}

impl Dirs {
    /// Resolves the directories of `app` from the process environment.
    pub fn from_env(app: &AppId) -> Dirs {
        Dirs::from_vars(app, std::env::vars_os().filter_map(|(k, v)| Some((k.into_string().ok()?, PathBuf::from(v)))))
    }

    /// Resolves the directories of `app` by asking `env` for each variable the layout reads.
    pub fn resolve(app: &AppId, env: impl Fn(&str) -> Option<PathBuf>) -> Dirs {
        let keys = SYSTEM_BASES.iter().map(|key| key.to_string()).chain(OVERRIDE_BASES.iter().map(|key| app.var(key)));
        Dirs::from_vars(app, keys.filter_map(|key| env(&key).map(|value| (key, value))))
    }

    /// Resolves the directories of `app` from explicit variables; empty values count as unset.
    pub fn from_vars(app: &AppId, vars: impl IntoIterator<Item = (String, PathBuf)>) -> Dirs {
        let vars: HashMap<String, PathBuf> = vars.into_iter().filter(|(_, v)| !v.as_os_str().is_empty()).collect();
        let get = |key: &str| vars.get(key).cloned();
        let home = get("HOME").unwrap_or_else(|| PathBuf::from("/"));
        let config_dir = home.join(".config").join(&app.name);
        let runtime_dir = get(&app.var("RUN_DIR"))
            .unwrap_or_else(|| get("XDG_RUNTIME_DIR").unwrap_or_else(|| PathBuf::from("/tmp")).join(&app.name));
        let xdg_data_home = get("XDG_DATA_HOME").unwrap_or_else(|| home.join(".local/share"));
        let xdg_cache_home = get("XDG_CACHE_HOME").unwrap_or_else(|| home.join(".cache"));
        let overrides = vars
            .iter()
            .filter(|(key, _)| {
                KINDS.iter().any(|kind| {
                    key.strip_prefix(&format!("{}_{kind}_", app.env_prefix)).is_some_and(|rest| rest != "DIR")
                })
            })
            .map(|(key, value)| (key.clone(), value.clone()))
            .collect();
        Dirs {
            settings: get(&app.var("SETTINGS")).unwrap_or_else(|| config_dir.join("settings.json")),
            cache_dir: get(&app.var("CACHE_DIR")).unwrap_or_else(|| home.join(".cache").join(&app.name)),
            state_dir: get(&app.var("STATE_DIR")).unwrap_or_else(|| home.join(".local/state").join(&app.name)),
            log_dir: get(&app.var("LOG_DIR")).unwrap_or_else(|| runtime_dir.join("logs")),
            data_dir: xdg_data_home.join(&app.name),
            ipc_socket: get("XDG_RUNTIME_DIR")
                .unwrap_or_else(|| PathBuf::from("/tmp"))
                .join(format!("{}.sock", app.name)),
            assets: PathBuf::new(),
            app: app.clone(),
            home,
            config_dir,
            runtime_dir,
            xdg_data_home,
            xdg_cache_home,
            overrides,
        }
    }

    /// Roots every directory of `app` under `root`, for tests and sandboxes.
    pub fn under(app: &AppId, root: &Path) -> Dirs {
        let mut dirs = Dirs::resolve(app, |key| (key == "HOME").then(|| root.join("home")));
        dirs.config_dir = root.join("config");
        dirs.settings = root.join("config/settings.json");
        dirs.cache_dir = root.join("cache");
        dirs.state_dir = root.join("state");
        dirs.runtime_dir = root.join("run");
        dirs.log_dir = root.join("run/logs");
        dirs.xdg_data_home = root.join("share");
        dirs.xdg_cache_home = root.join("xdg-cache");
        dirs.data_dir = root.join("share").join(&app.name);
        dirs.ipc_socket = root.join(format!("{}.sock", app.name));
        dirs.assets = root.join("assets");
        dirs
    }

    /// Directory for `widget`, honouring the per-widget override variable.
    pub fn cache_for(&self, widget: &str) -> PathBuf {
        self.scoped(&self.cache_dir, "CACHE", widget)
    }

    /// Directory for `widget`, honouring the per-widget override variable.
    pub fn state_for(&self, widget: &str) -> PathBuf {
        self.scoped(&self.state_dir, "STATE", widget)
    }

    /// Directory for `widget`, honouring the per-widget override variable.
    pub fn run_for(&self, widget: &str) -> PathBuf {
        self.scoped(&self.runtime_dir, "RUN", widget)
    }

    /// Directory for `widget`, honouring the per-widget override variable.
    pub fn log_for(&self, widget: &str) -> PathBuf {
        self.scoped(&self.log_dir, "LOG", widget)
    }

    /// Lyrics cache under the XDG cache home.
    pub fn lyrics_cache(&self) -> PathBuf {
        self.xdg_cache_home.join(&self.app.name).join("lyrics")
    }

    fn scoped(&self, base: &Path, kind: &str, widget: &str) -> PathBuf {
        if widget.is_empty() || widget == self.app.name || base.to_string_lossy().ends_with(&format!("/{widget}")) {
            return base.to_path_buf();
        }
        match self.overrides.get(&format!("{}_{kind}_{}", self.app.env_prefix, widget.to_uppercase())) {
            Some(path) => path.clone(),
            None => base.join(widget),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dirs(pairs: &[(&str, &str)]) -> Dirs {
        Dirs::from_vars(&app(), pairs.iter().map(|(k, v)| (k.to_string(), PathBuf::from(v))))
    }

    fn app() -> AppId {
        AppId::new("demo", "DM")
    }

    fn p(text: &str) -> PathBuf {
        PathBuf::from(text)
    }

    #[test]
    fn nothing_set_uses_home_and_tmp() {
        let d = dirs(&[("HOME", "/home/u")]);
        assert_eq!(d.settings, p("/home/u/.config/demo/settings.json"));
        assert_eq!(d.cache_dir, p("/home/u/.cache/demo"));
        assert_eq!(d.state_dir, p("/home/u/.local/state/demo"));
        assert_eq!(d.runtime_dir, p("/tmp/demo"));
        assert_eq!(d.log_dir, p("/tmp/demo/logs"));
        assert_eq!(d.ipc_socket, p("/tmp/demo.sock"));
        assert_eq!(d.data_dir, p("/home/u/.local/share/demo"));
    }

    #[test]
    fn the_runtime_dir_follows_xdg_runtime_dir() {
        let d = dirs(&[("HOME", "/home/u"), ("XDG_RUNTIME_DIR", "/run/user/1")]);
        assert_eq!(d.runtime_dir, p("/run/user/1/demo"));
        assert_eq!(d.log_dir, p("/run/user/1/demo/logs"));
        assert_eq!(d.run_for("lock"), p("/run/user/1/demo/lock"));
        assert_eq!(d.ipc_socket, p("/run/user/1/demo.sock"));
    }

    #[test]
    fn xdg_cache_state_and_config_homes_are_ignored() {
        let d = dirs(&[
            ("HOME", "/home/u"),
            ("XDG_CACHE_HOME", "/c"),
            ("XDG_STATE_HOME", "/s"),
            ("XDG_CONFIG_HOME", "/cfg"),
        ]);
        assert_eq!(d.cache_dir, p("/home/u/.cache/demo"));
        assert_eq!(d.state_dir, p("/home/u/.local/state/demo"));
        assert_eq!(d.settings, p("/home/u/.config/demo/settings.json"));
    }

    #[test]
    fn each_base_override_wins() {
        let d = dirs(&[
            ("HOME", "/home/u"),
            ("DM_SETTINGS", "/x/s.json"),
            ("DM_CACHE_DIR", "/x/cache"),
            ("DM_STATE_DIR", "/x/state"),
            ("DM_RUN_DIR", "/x/run"),
            ("DM_LOG_DIR", "/x/log"),
        ]);
        assert_eq!(d.settings, p("/x/s.json"));
        assert_eq!(d.cache_dir, p("/x/cache"));
        assert_eq!(d.state_dir, p("/x/state"));
        assert_eq!(d.runtime_dir, p("/x/run"));
        assert_eq!(d.log_dir, p("/x/log"));
        assert_eq!(d.cache_for("weather"), p("/x/cache/weather"));
        assert_eq!(d.state_for("quickactions"), p("/x/state/quickactions"));
        assert_eq!(d.run_for("music"), p("/x/run/music"));
        assert_eq!(d.log_for("applauncher"), p("/x/log/applauncher"));
    }

    #[test]
    fn the_log_dir_follows_the_run_dir_override() {
        let d = dirs(&[("HOME", "/h"), ("DM_RUN_DIR", "/x/run")]);
        assert_eq!(d.log_dir, p("/x/run/logs"));
    }

    #[test]
    fn per_widget_overrides_win_over_the_subdirectory() {
        let d = dirs(&[
            ("HOME", "/h"),
            ("DM_CACHE_WEATHER", "/w"),
            ("DM_STATE_FOCUSTIME", "/f"),
            ("DM_RUN_MUSIC", "/m"),
            ("DM_LOG_APPLAUNCHER", "/l"),
        ]);
        assert_eq!(d.cache_for("weather"), p("/w"));
        assert_eq!(d.state_for("focustime"), p("/f"));
        assert_eq!(d.run_for("music"), p("/m"));
        assert_eq!(d.log_for("applauncher"), p("/l"));
        assert_eq!(d.cache_for("wallpaper"), p("/h/.cache/demo/wallpaper"));
    }

    #[test]
    fn a_base_that_already_ends_in_the_widget_is_used_directly() {
        let d = dirs(&[("HOME", "/h"), ("DM_RUN_DIR", "/r/lock")]);
        assert_eq!(d.run_for("lock"), p("/r/lock"));
        assert_eq!(d.run_for("music"), p("/r/lock/music"));
    }

    #[test]
    fn the_empty_and_the_application_name_give_the_base() {
        let d = dirs(&[("HOME", "/h")]);
        assert_eq!(d.state_for(""), d.state_dir);
        assert_eq!(d.state_for("demo"), d.state_dir);
        assert_eq!(d.cache_for("demo"), d.cache_dir);
    }

    #[test]
    fn widget_names_are_upper_cased_for_the_override() {
        let d = dirs(&[("HOME", "/h"), ("DM_CACHE_MY-WIDGET", "/m")]);
        assert_eq!(d.cache_for("my-widget"), p("/m"));
    }

    #[test]
    fn nested_widget_names_join_as_a_path() {
        let d = dirs(&[("HOME", "/h")]);
        assert_eq!(d.state_for("widgets/DP-1"), p("/h/.local/state/demo/widgets/DP-1"));
    }

    #[test]
    fn empty_values_count_as_unset() {
        let d = dirs(&[("HOME", "/h"), ("DM_CACHE_DIR", ""), ("DM_RUN_WEATHER", "")]);
        assert_eq!(d.cache_dir, p("/h/.cache/demo"));
        assert_eq!(d.run_for("weather"), p("/tmp/demo/weather"));
    }

    #[test]
    fn no_home_falls_back_to_the_root() {
        let d = dirs(&[]);
        assert_eq!(d.cache_dir, p("/.cache/demo"));
    }

    #[test]
    fn the_lyrics_cache_and_the_data_dir_follow_the_xdg_homes_like_the_qml() {
        let d = dirs(&[("HOME", "/h")]);
        assert_eq!(d.lyrics_cache(), p("/h/.cache/demo/lyrics"));
        let d = dirs(&[("HOME", "/h"), ("XDG_CACHE_HOME", "/c"), ("XDG_DATA_HOME", "/d")]);
        assert_eq!(d.lyrics_cache(), p("/c/demo/lyrics"));
        assert_eq!(d.data_dir, p("/d/demo"));
    }

    #[test]
    fn resolve_reads_only_the_known_variables() {
        let d = Dirs::resolve(&app(), |key| match key {
            "HOME" => Some(p("/h")),
            "DM_STATE_DIR" => Some(p("/s")),
            _ => None,
        });
        assert_eq!(d.state_dir, p("/s"));
        assert_eq!(d.cache_dir, p("/h/.cache/demo"));
    }

    #[test]
    fn under_roots_everything_in_one_directory() {
        let d = Dirs::under(&app(), Path::new("/t"));
        assert_eq!(d.cache_for("weather"), p("/t/cache/weather"));
        assert_eq!(d.run_for("lock"), p("/t/run/lock"));
        assert_eq!(d.state_dir, p("/t/state"));
    }
}
