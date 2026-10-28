use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

#[derive(Debug, Clone, PartialEq)]
enum DirKind {
    Fixed,
    Scalable,
    Threshold,
}

#[derive(Debug, Clone)]
struct IconDir {
    path: String,
    size: i32,
    scale: i32,
    kind: DirKind,
    min: i32,
    max: i32,
    threshold: i32,
}

impl IconDir {
    fn matches(&self, size: i32, scale: i32) -> bool {
        if self.scale != scale {
            return false;
        }
        match self.kind {
            DirKind::Fixed => self.size == size,
            DirKind::Scalable => self.min <= size && size <= self.max,
            DirKind::Threshold => self.size - self.threshold <= size && size <= self.size + self.threshold,
        }
    }

    fn distance(&self, size: i32, scale: i32) -> i32 {
        let wanted = size * scale;
        let own = self.size * self.scale;
        match self.kind {
            DirKind::Fixed => (own - wanted).abs(),
            DirKind::Scalable => {
                if wanted < self.min * self.scale {
                    self.min * self.scale - wanted
                } else if wanted > self.max * self.scale {
                    wanted - self.max * self.scale
                } else {
                    0
                }
            }
            DirKind::Threshold => {
                if wanted < (self.size - self.threshold) * self.scale {
                    (self.size - self.threshold) * self.scale - wanted
                } else if wanted > (self.size + self.threshold) * self.scale {
                    wanted - (self.size + self.threshold) * self.scale
                } else {
                    0
                }
            }
        }
    }
}

#[derive(Debug, Clone, Default)]
struct ThemeIndex {
    inherits: Vec<String>,
    dirs: Vec<IconDir>,
}

fn parse_index(text: &str) -> ThemeIndex {
    let mut sections: Vec<(String, HashMap<String, String>)> = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some(name) = line.strip_prefix('[').and_then(|l| l.strip_suffix(']')) {
            sections.push((name.to_owned(), HashMap::new()));
        } else if let Some((k, v)) = line.split_once('=')
            && let Some((_, map)) = sections.last_mut()
        {
            map.entry(k.trim().to_owned()).or_insert_with(|| v.trim().to_owned());
        }
    }
    let list = |v: Option<&String>| -> Vec<String> {
        v.map(|v| v.split(',').map(|s| s.trim().to_owned()).filter(|s| !s.is_empty()).collect()).unwrap_or_default()
    };
    let Some((_, header)) = sections.iter().find(|(n, _)| n == "Icon Theme") else { return ThemeIndex::default() };
    let mut names = list(header.get("Directories"));
    names.extend(list(header.get("ScaledDirectories")));
    let int = |m: &HashMap<String, String>, k: &str| m.get(k).and_then(|v| v.parse::<i32>().ok());
    let mut dirs = Vec::new();
    for name in names {
        let Some((_, m)) = sections.iter().find(|(n, _)| *n == name) else { continue };
        let Some(size) = int(m, "Size") else { continue };
        let kind = match m.get("Type").map(String::as_str) {
            Some("Fixed") => DirKind::Fixed,
            Some("Scalable") => DirKind::Scalable,
            _ => DirKind::Threshold,
        };
        dirs.push(IconDir {
            path: name,
            size,
            scale: int(m, "Scale").unwrap_or(1),
            kind,
            min: int(m, "MinSize").unwrap_or(size),
            max: int(m, "MaxSize").unwrap_or(size),
            threshold: int(m, "Threshold").unwrap_or(2),
        });
    }
    ThemeIndex { inherits: list(header.get("Inherits")), dirs }
}

/// Freedesktop icon theme lookup with a per-theme index and result cache.
pub struct IconLookup {
    bases: Vec<PathBuf>,
    pixmaps: Vec<PathBuf>,
    theme: Mutex<String>,
    indexes: Mutex<HashMap<String, ThemeIndex>>,
    cache: Mutex<HashMap<(String, i32), Option<PathBuf>>>,
}

impl IconLookup {
    /// Lookup over explicit icon base directories and pixmap directories using `theme`.
    pub fn new(bases: Vec<PathBuf>, pixmaps: Vec<PathBuf>, theme: &str) -> Self {
        IconLookup {
            bases,
            pixmaps,
            theme: Mutex::new(theme.to_owned()),
            indexes: Mutex::new(HashMap::new()),
            cache: Mutex::new(HashMap::new()),
        }
    }

    /// Lookup configured from the XDG directories and the GTK icon theme setting, falling back to `hicolor`.
    pub fn from_environment() -> Self {
        let home = std::env::var_os("HOME").map(PathBuf::from);
        let data_home = std::env::var_os("XDG_DATA_HOME")
            .map(PathBuf::from)
            .filter(|p| p.is_absolute())
            .or_else(|| home.as_ref().map(|h| h.join(".local/share")));
        let data_dirs = std::env::var("XDG_DATA_DIRS")
            .ok()
            .filter(|v| !v.is_empty())
            .unwrap_or_else(|| "/usr/local/share:/usr/share".to_owned());
        let mut bases = Vec::new();
        let mut pixmaps = Vec::new();
        if let Some(h) = &home {
            bases.push(h.join(".icons"));
        }
        if let Some(d) = &data_home {
            bases.push(d.join("icons"));
            pixmaps.push(d.join("pixmaps"));
        }
        for dir in data_dirs.split(':').filter(|d| !d.is_empty()) {
            bases.push(Path::new(dir).join("icons"));
            pixmaps.push(Path::new(dir).join("pixmaps"));
        }
        let config = std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .filter(|p| p.is_absolute())
            .or_else(|| home.as_ref().map(|h| h.join(".config")));
        let theme = config
            .iter()
            .flat_map(|c| [c.join("gtk-3.0/settings.ini"), c.join("gtk-4.0/settings.ini")])
            .find_map(|p| gtk_icon_theme(&p))
            .unwrap_or_else(|| "hicolor".to_owned());
        IconLookup::new(bases, pixmaps, &theme)
    }

    /// The current theme name.
    pub fn theme(&self) -> String {
        self.theme.lock().unwrap().clone()
    }

    /// Switches theme and clears the cache when it changes.
    pub fn set_theme(&self, theme: &str) {
        let mut current = self.theme.lock().unwrap();
        if *current != theme {
            *current = theme.to_owned();
            self.cache.lock().unwrap().clear();
        }
    }

    /// Finds the file of icon `name` closest to `size` pixels.
    pub fn lookup(&self, name: &str, size: u32) -> Option<PathBuf> {
        let size = size.max(1) as i32;
        let key = (name.to_owned(), size);
        if let Some(hit) = self.cache.lock().unwrap().get(&key) {
            return hit.clone();
        }
        let found = self.search(name, size);
        self.cache.lock().unwrap().insert(key, found.clone());
        found
    }

    fn index(&self, theme: &str) -> Option<ThemeIndex> {
        let mut indexes = self.indexes.lock().unwrap();
        if let Some(index) = indexes.get(theme) {
            return Some(index.clone());
        }
        let text = self.bases.iter().find_map(|b| std::fs::read_to_string(b.join(theme).join("index.theme")).ok())?;
        let index = parse_index(&text);
        indexes.insert(theme.to_owned(), index.clone());
        Some(index)
    }

    fn chain(&self, theme: &str) -> Vec<String> {
        let mut out: Vec<String> = Vec::new();
        let mut stack = vec![theme.to_owned()];
        while let Some(t) = stack.pop() {
            if out.contains(&t) {
                continue;
            }
            if let Some(index) = self.index(&t) {
                for parent in index.inherits.iter().rev() {
                    stack.push(parent.clone());
                }
            }
            out.push(t);
        }
        if !out.iter().any(|t| t == "hicolor") {
            out.push("hicolor".to_owned());
        }
        out
    }

    fn search(&self, name: &str, size: i32) -> Option<PathBuf> {
        let chain = self.chain(&self.theme());
        for theme in &chain {
            if let Some(path) = self.in_theme(theme, name, size) {
                return Some(path);
            }
        }
        for dir in &self.pixmaps {
            for ext in ["svg", "png", "xpm"] {
                let p = dir.join(format!("{name}.{ext}"));
                if p.is_file() {
                    return Some(p);
                }
            }
        }
        None
    }

    fn in_theme(&self, theme: &str, name: &str, size: i32) -> Option<PathBuf> {
        let index = self.index(theme)?;
        let find = |dir: &IconDir| -> Option<PathBuf> {
            for base in &self.bases {
                for ext in ["svg", "png", "xpm"] {
                    let p = base.join(theme).join(&dir.path).join(format!("{name}.{ext}"));
                    if p.is_file() {
                        return Some(p);
                    }
                }
            }
            None
        };
        if let Some(path) = index.dirs.iter().filter(|d| d.matches(size, 1)).find_map(find) {
            return Some(path);
        }
        let mut ranked: Vec<(&IconDir, i32)> = index.dirs.iter().map(|d| (d, d.distance(size, 1))).collect();
        ranked.sort_by_key(|(_, distance)| *distance);
        ranked.into_iter().find_map(|(d, _)| find(d))
    }
}

fn gtk_icon_theme(path: &Path) -> Option<String> {
    let text = std::fs::read_to_string(path).ok()?;
    text.lines().find_map(|l| {
        let (k, v) = l.split_once('=')?;
        (k.trim() == "gtk-icon-theme-name").then(|| v.trim().trim_matches('"').to_owned()).filter(|v| !v.is_empty())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(path: &Path, text: &str) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    }

    fn lookup(dir: &Path) -> IconLookup {
        write(
            &dir.join("icons/Fancy/index.theme"),
            "[Icon Theme]\nInherits=base\nDirectories=48x48/apps\n\n[48x48/apps]\nSize=48\nType=Fixed\n",
        );
        write(
            &dir.join("icons/base/index.theme"),
            "[Icon Theme]\nDirectories=16x16/apps,scalable/apps\n\n[16x16/apps]\nSize=16\nType=Fixed\n\n[scalable/apps]\nSize=64\nMinSize=8\nMaxSize=512\nType=Scalable\n",
        );
        IconLookup::new(vec![dir.join("icons")], vec![dir.join("pixmaps")], "Fancy")
    }

    #[test]
    fn exact_and_inherited_lookup() {
        let dir = tempfile::tempdir().unwrap();
        let icons = lookup(dir.path());
        write(&dir.path().join("icons/Fancy/48x48/apps/fox.png"), "");
        write(&dir.path().join("icons/base/scalable/apps/bird.svg"), "");
        write(&dir.path().join("icons/base/16x16/apps/bird.png"), "");
        assert_eq!(icons.lookup("fox", 48), Some(dir.path().join("icons/Fancy/48x48/apps/fox.png")));
        assert_eq!(icons.lookup("bird", 128), Some(dir.path().join("icons/base/scalable/apps/bird.svg")));
        assert_eq!(icons.lookup("bird", 16), Some(dir.path().join("icons/base/16x16/apps/bird.png")));
        assert_eq!(icons.lookup("missing", 32), None);
    }

    #[test]
    fn closest_size_and_pixmap_fallback() {
        let dir = tempfile::tempdir().unwrap();
        let icons = lookup(dir.path());
        write(&dir.path().join("icons/Fancy/48x48/apps/only.png"), "");
        write(&dir.path().join("pixmaps/legacy.png"), "");
        assert_eq!(icons.lookup("only", 22), Some(dir.path().join("icons/Fancy/48x48/apps/only.png")));
        assert_eq!(icons.lookup("legacy", 32), Some(dir.path().join("pixmaps/legacy.png")));
    }

    #[test]
    fn changing_theme_clears_the_cache() {
        let dir = tempfile::tempdir().unwrap();
        let icons = lookup(dir.path());
        write(&dir.path().join("icons/base/16x16/apps/x.png"), "");
        assert!(icons.lookup("x", 16).is_some());
        icons.set_theme("nonexistent");
        assert_eq!(icons.theme(), "nonexistent");
        assert!(icons.lookup("x", 16).is_none());
    }
}
