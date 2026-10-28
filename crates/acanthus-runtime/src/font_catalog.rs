use std::collections::HashMap;
use std::path::{Path, PathBuf};

use cosmic_text::fontdb;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FontEntry {
    pub name: String,
    pub path: PathBuf,
}

#[derive(Debug, Clone, Default)]
pub struct FontCatalog {
    by_key: HashMap<String, FontEntry>,
    names: Vec<String>,
}

const HIDDEN_FAMILY: &str = "symbols nerd font";

fn is_font_file(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| matches!(e.to_ascii_lowercase().as_str(), "ttf" | "otf" | "ttc"))
}

impl FontCatalog {
    pub fn scan(dirs: &[PathBuf]) -> FontCatalog {
        let mut files: Vec<PathBuf> = Vec::new();
        for dir in dirs {
            collect(dir, &mut files);
        }
        files.sort();
        let mut catalog = FontCatalog::default();
        for file in files {
            let mut db = fontdb::Database::new();
            if db.load_font_file(&file).is_err() {
                continue;
            }
            for face in db.faces() {
                let family =
                    face.families.first().map(|(n, _)| n.clone()).unwrap_or_else(|| {
                        file.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default()
                    });
                let regular = face.style == fontdb::Style::Normal && face.weight == fontdb::Weight::NORMAL;
                let name = if regular { family } else { format!("{family} {}", style_label(face)) };
                catalog.insert(FontEntry { name, path: file.clone() });
            }
        }
        catalog
    }

    fn insert(&mut self, entry: FontEntry) {
        if entry.name.to_lowercase().contains(HIDDEN_FAMILY) {
            return;
        }
        let key = entry.name.to_lowercase();
        if !self.by_key.contains_key(&key) {
            self.names.push(entry.name.clone());
        }
        self.by_key.insert(key, entry);
    }

    pub fn names(&self) -> &[String] {
        &self.names
    }

    pub fn find(&self, family: &str) -> Option<&FontEntry> {
        self.by_key.get(&family.to_lowercase())
    }

    pub fn len(&self) -> usize {
        self.names.len()
    }

    pub fn is_empty(&self) -> bool {
        self.names.is_empty()
    }
}

fn style_label(face: &fontdb::FaceInfo) -> String {
    let weight = match face.weight.0 {
        0..=199 => "Thin",
        200..=299 => "ExtraLight",
        300..=399 => "Light",
        400..=499 => "",
        500..=599 => "Medium",
        600..=699 => "SemiBold",
        700..=799 => "Bold",
        800..=899 => "ExtraBold",
        _ => "Black",
    };
    let slant = match face.style {
        fontdb::Style::Normal => "",
        fontdb::Style::Italic => "Italic",
        fontdb::Style::Oblique => "Oblique",
    };
    [weight, slant].iter().filter(|s| !s.is_empty()).copied().collect::<Vec<_>>().join(" ")
}

fn collect(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect(&path, out);
        } else if is_font_file(&path) {
            out.push(path);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bundled_fonts_are_cataloged_without_the_icon_font() {
        let paths = crate::paths::test_paths();
        let catalog = FontCatalog::scan(&[paths.assets.join("fonts")]);
        assert!(catalog.len() >= 2, "{}", catalog.len());
        assert!(catalog.names().iter().all(|n| !n.to_lowercase().contains("symbols nerd font")));
        let entry = catalog.find("jetbrains mono").expect("JetBrains Mono");
        assert!(entry.path.ends_with("JetBrainsMono-Regular.ttf"));
    }

    #[test]
    fn missing_directories_are_ignored() {
        assert!(FontCatalog::scan(&[PathBuf::from("/nonexistent/fonts")]).is_empty());
    }
}
