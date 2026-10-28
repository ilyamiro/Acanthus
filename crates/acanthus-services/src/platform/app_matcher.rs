use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use crate::platform::desktop::DesktopEntry;

const WRAPPER_EXECS: &[&str] = &[
    "env",
    "sh",
    "bash",
    "zsh",
    "fish",
    "flatpak",
    "snap",
    "gtk-launch",
    "xdg-open",
    "gio",
    "systemd-run",
    "uwsm",
    "uwsm-app",
    "nix",
    "nix-shell",
    "java",
    "python",
    "python3",
    "wine",
    "electron",
    "dex",
    "sudo",
    "pkexec",
];

pub fn canonical_id(id: &str) -> String {
    let mut s = id.trim().to_lowercase();
    while let Some(stripped) = s.strip_suffix(".desktop") {
        s = stripped.to_owned();
    }
    s
}

pub fn normalize_key(value: &str) -> String {
    value.trim().to_lowercase()
}

fn put(map: &mut HashMap<String, String>, key: String, id: &str) {
    if !key.is_empty() {
        map.entry(key).or_insert_with(|| id.to_owned());
    }
}

pub struct AppMatcher {
    by_id: HashMap<String, Rc<DesktopEntry>>,
    keys: HashMap<String, String>,
    cache: RefCell<HashMap<String, String>>,
}

impl AppMatcher {
    pub fn new(entries: &[Rc<DesktopEntry>]) -> AppMatcher {
        let mut by_id: HashMap<String, Rc<DesktopEntry>> = HashMap::new();
        let (mut id_keys, mut class_keys, mut exec_keys, mut tail_keys, mut name_keys) =
            (HashMap::new(), HashMap::new(), HashMap::new(), HashMap::new(), HashMap::new());

        for entry in entries.iter().filter(|e| !e.id.is_empty() && !e.no_display) {
            let cid = canonical_id(&entry.id);
            by_id.entry(cid.clone()).or_insert_with(|| entry.clone());
            put(&mut id_keys, cid.clone(), &cid);
            put(&mut class_keys, normalize_key(&entry.startup_wm_class), &cid);
            if let Some(command) = crate::platform::desktop::split_exec(&entry.exec).first() {
                let base = normalize_key(command.rsplit('/').next().unwrap_or(command));
                if !base.is_empty() && !WRAPPER_EXECS.contains(&base.as_str()) {
                    put(&mut exec_keys, base, &cid);
                }
            }
            if let Some((_, tail)) = cid.rsplit_once('.') {
                put(&mut tail_keys, tail.to_owned(), &cid);
            }
            put(&mut name_keys, normalize_key(&entry.name), &cid);
        }

        let mut keys = HashMap::new();
        for table in [name_keys, tail_keys, exec_keys, class_keys, id_keys] {
            keys.extend(table);
        }
        AppMatcher { by_id, keys, cache: RefCell::default() }
    }

    pub fn resolve(&self, app_id: &str) -> String {
        let key = normalize_key(app_id);
        if key.is_empty() {
            return String::new();
        }
        if let Some(cached) = self.cache.borrow().get(&key) {
            return cached.clone();
        }
        let cid_key = canonical_id(&key);
        let tail = cid_key.rsplit('.').next().unwrap_or(&cid_key).to_owned();
        let found = self
            .keys
            .get(&key)
            .or_else(|| self.keys.get(&cid_key))
            .or_else(|| (tail != cid_key).then(|| self.keys.get(&tail)).flatten())
            .cloned()
            .or_else(|| self.heuristic(&cid_key));
        let resolved = found.unwrap_or(cid_key);
        self.cache.borrow_mut().insert(key, resolved.clone());
        resolved
    }

    fn heuristic(&self, cid_key: &str) -> Option<String> {
        let variants = [cid_key.replace('-', "."), cid_key.replace('_', "-"), cid_key.replace('-', "_")];
        variants
            .iter()
            .find_map(|v| self.keys.get(v))
            .cloned()
            .or_else(|| self.by_id.keys().find(|id| id.rsplit('.').next() == Some(cid_key)).cloned())
    }

    pub fn entry(&self, id: &str) -> Option<Rc<DesktopEntry>> {
        let cid = canonical_id(id);
        (!cid.is_empty()).then(|| self.by_id.get(&cid).cloned()).flatten()
    }

    pub fn len(&self) -> usize {
        self.by_id.len()
    }

    pub fn is_empty(&self) -> bool {
        self.by_id.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(id: &str, name: &str, exec: &str, class: &str) -> Rc<DesktopEntry> {
        Rc::new(DesktopEntry {
            id: id.into(),
            name: name.into(),
            exec: exec.into(),
            startup_wm_class: class.into(),
            ..Default::default()
        })
    }

    fn matcher() -> AppMatcher {
        AppMatcher::new(&[
            entry("org.gnome.Nautilus.desktop", "Files", "nautilus --new-window", ""),
            entry("firefox.desktop", "Firefox", "/usr/bin/firefox %u", "firefox"),
            entry("steam.desktop", "Steam", "env STEAM_RUNTIME=1 steam %U", "Steam"),
            entry("kitty.desktop", "kitty", "kitty", "kitty"),
            Rc::new(DesktopEntry {
                id: "hidden.desktop".into(),
                name: "Hidden".into(),
                exec: "hidden".into(),
                no_display: true,
                ..Default::default()
            }),
        ])
    }

    #[test]
    fn canonical_ids_strip_desktop_suffixes_and_case() {
        assert_eq!(canonical_id("Firefox.desktop"), "firefox");
        assert_eq!(canonical_id(" org.Gnome.X.desktop.desktop "), "org.gnome.x");
        assert_eq!(canonical_id(""), "");
    }

    #[test]
    fn resolves_by_id_class_exec_name_and_tail() {
        let m = matcher();
        assert_eq!(m.resolve("firefox"), "firefox");
        assert_eq!(m.resolve("Firefox.desktop"), "firefox");
        assert_eq!(m.resolve("Steam"), "steam");
        assert_eq!(m.resolve("nautilus"), "org.gnome.nautilus");
        assert_eq!(m.resolve("org.gnome.Nautilus"), "org.gnome.nautilus");
        assert_eq!(m.resolve("Files"), "org.gnome.nautilus");
    }

    #[test]
    fn wrapper_executables_never_become_match_keys() {
        let m = matcher();
        assert_eq!(m.resolve("env"), "env");
        assert_ne!(m.resolve("env"), "steam");
    }

    #[test]
    fn unknown_apps_fall_back_to_their_canonical_name_and_hidden_entries_are_skipped() {
        let m = matcher();
        assert_eq!(m.resolve("SomeUnknownApp"), "someunknownapp");
        assert_eq!(m.resolve("hidden"), "hidden");
        assert!(m.entry("hidden").is_none());
        assert_eq!(m.resolve(""), "");
        assert_eq!(m.len(), 4);
    }

    #[test]
    fn entry_lookup_and_hyphen_dot_heuristics() {
        let m = matcher();
        assert_eq!(m.entry("Firefox.desktop").unwrap().name, "Firefox");
        assert!(m.entry("nope").is_none());
        let dotted = AppMatcher::new(&[entry("org.kde.dolphin.desktop", "Dolphin", "dolphin", "")]);
        assert_eq!(dotted.resolve("org-kde-dolphin"), "org.kde.dolphin");
    }
}
