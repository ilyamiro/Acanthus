use std::ops::{Deref, DerefMut};
use std::path::PathBuf;

use acanthus_io::{AppId, Dirs};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Paths {
    dirs: Dirs,
}

impl Deref for Paths {
    type Target = Dirs;

    fn deref(&self) -> &Dirs {
        &self.dirs
    }
}

impl DerefMut for Paths {
    fn deref_mut(&mut self) -> &mut Dirs {
        &mut self.dirs
    }
}

impl Paths {
    /// Resolves the paths of `app` from the process environment.
    pub fn from_env(app: &AppId) -> Paths {
        Paths::from_env_with(app, &[])
    }

    /// Like [`Paths::from_env`], with `fallbacks` as extra asset roots (for example a source checkout) tried after
    /// the installed location; the first one that contains a `languages` directory wins.
    pub fn from_env_with(app: &AppId, fallbacks: &[PathBuf]) -> Paths {
        let env = |key: &str| std::env::var_os(key).filter(|v| !v.is_empty()).map(PathBuf::from);
        let mut dirs = Dirs::from_env(app);
        dirs.assets = resolve_assets(app, &env, fallbacks);
        Paths { dirs }
    }

    /// Resolves the paths of `app` by asking `env` for each variable.
    pub fn resolve(app: &AppId, env: impl Fn(&str) -> Option<PathBuf>) -> Paths {
        Paths::resolve_with(app, env, &[])
    }

    /// Like [`Paths::resolve`], with `fallbacks` as extra asset roots.
    pub fn resolve_with(app: &AppId, env: impl Fn(&str) -> Option<PathBuf>, fallbacks: &[PathBuf]) -> Paths {
        let mut dirs = Dirs::resolve(app, &env);
        dirs.assets = resolve_assets(app, &env, fallbacks);
        Paths { dirs }
    }

    pub fn dirs(&self) -> Dirs {
        self.dirs.clone()
    }

    pub fn language_file(&self, lang: &str) -> PathBuf {
        self.assets.join("languages").join(format!("{lang}.json"))
    }

    pub fn languages_dir(&self) -> PathBuf {
        self.assets.join("languages")
    }

    pub fn themes_dir(&self) -> PathBuf {
        self.assets.join("themes")
    }
}

#[cfg(test)]
pub(crate) fn test_app() -> AppId {
    AppId::new("demo", "DM")
}

#[cfg(test)]
pub(crate) fn test_paths() -> Paths {
    Paths::resolve_with(&test_app(), |_| None, &[repo_assets()])
}

#[cfg(test)]
fn repo_assets() -> PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../assets")
}

fn resolve_assets(app: &AppId, env: &impl Fn(&str) -> Option<PathBuf>, fallbacks: &[PathBuf]) -> PathBuf {
    let upper = app.name.to_uppercase();
    if let Some(dir) = env(&format!("{upper}_ASSETS")) {
        return dir;
    }
    if let Some(dir) = env(&format!("{upper}_DIR")) {
        return dir.join("assets");
    }
    let installed = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.join("../share").join(&app.name).join("assets")));
    installed
        .into_iter()
        .chain(fallbacks.iter().cloned())
        .find(|p| p.join("languages").is_dir())
        .unwrap_or_else(|| PathBuf::from("assets"))
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;

    fn app() -> AppId {
        test_app()
    }

    fn env_of(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<PathBuf> {
        let map: HashMap<String, PathBuf> = pairs.iter().map(|(k, v)| (k.to_string(), PathBuf::from(v))).collect();
        move |k| map.get(k).cloned()
    }

    #[test]
    fn defaults_derive_from_home_like_caching_sh() {
        let p = Paths::resolve(&app(), env_of(&[("HOME", "/home/u"), ("XDG_RUNTIME_DIR", "/run/user/1")]));
        assert_eq!(p.settings, PathBuf::from("/home/u/.config/demo/settings.json"));
        assert_eq!(p.cache_dir, PathBuf::from("/home/u/.cache/demo"));
        assert_eq!(p.state_dir, PathBuf::from("/home/u/.local/state/demo"));
        assert_eq!(p.runtime_dir, PathBuf::from("/run/user/1/demo"));
        assert_eq!(p.log_dir, PathBuf::from("/run/user/1/demo/logs"));
        assert_eq!(p.ipc_socket, PathBuf::from("/run/user/1/demo.sock"));
    }

    #[test]
    fn without_a_runtime_dir_the_run_dir_is_in_tmp() {
        let p = Paths::resolve(&app(), env_of(&[("HOME", "/home/u")]));
        assert_eq!(p.runtime_dir, PathBuf::from("/tmp/demo"));
    }

    #[test]
    fn explicit_environment_wins() {
        let p = Paths::resolve(
            &app(),
            env_of(&[
                ("HOME", "/home/u"),
                ("XDG_CONFIG_HOME", "/cfg"),
                ("DM_SETTINGS", "/tmp/s.json"),
                ("DEMO_DIR", "/opt/serp"),
            ]),
        );
        assert_eq!(p.config_dir, PathBuf::from("/home/u/.config/demo"));
        assert_eq!(p.settings, PathBuf::from("/tmp/s.json"));
        assert_eq!(p.assets, PathBuf::from("/opt/serp/assets"));
        assert_eq!(p.language_file("de"), PathBuf::from("/opt/serp/assets/languages/de.json"));
    }

    #[test]
    fn repository_assets_are_found_by_default() {
        let p = Paths::resolve_with(&app(), env_of(&[("HOME", "/home/u")]), &[repo_assets()]);
        assert!(p.languages_dir().join("en.json").is_file(), "{:?}", p.assets);
    }
}
