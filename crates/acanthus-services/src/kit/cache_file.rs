/// A JSON file used as a cache, written atomically.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CacheFile {
    path: std::path::PathBuf,
}

impl CacheFile {
    /// A cache at `path`.
    pub fn new(path: std::path::PathBuf) -> CacheFile {
        CacheFile { path }
    }

    /// The file path.
    pub fn path(&self) -> &std::path::Path {
        &self.path
    }

    /// The parsed content, or `None` when the file is missing or invalid.
    pub fn read_json(&self) -> Option<serde_json::Value> {
        serde_json::from_str(std::fs::read_to_string(&self.path).ok()?.trim()).ok()
    }

    /// Writes `value` atomically.
    pub fn write_json(&self, value: &serde_json::Value) {
        crate::kit::fsx::write_atomic("cache", &self.path, value.to_string().as_bytes());
    }

    /// Marks the cache as fresh without changing it.
    pub fn touch(&self) {
        crate::kit::fsx::touch("cache", &self.path);
    }

    /// Seconds since the file was last modified, or `None` when it does not exist.
    pub fn age_seconds(&self) -> Option<u64> {
        let modified = std::fs::metadata(&self.path).ok()?.modified().ok()?;
        Some(std::time::SystemTime::now().duration_since(modified).map(|d| d.as_secs()).unwrap_or(0))
    }

    /// True when the file exists.
    pub fn exists(&self) -> bool {
        self.path.exists()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cache_file_round_trips_json_and_tracks_age() {
        let dir = tempfile::tempdir().unwrap();
        let cache = CacheFile::new(dir.path().join("a").join("b.json"));
        assert!(!cache.exists());
        assert!(cache.read_json().is_none());
        assert!(cache.age_seconds().is_none());
        cache.write_json(&serde_json::json!({"x": 1}));
        assert_eq!(cache.read_json().unwrap()["x"], 1);
        assert!(cache.age_seconds().unwrap() < 5);
        let old = std::time::SystemTime::now() - std::time::Duration::from_secs(500);
        std::fs::OpenOptions::new().write(true).open(cache.path()).unwrap().set_modified(old).unwrap();
        assert!(cache.age_seconds().unwrap() >= 500);
        cache.touch();
        assert!(cache.age_seconds().unwrap() < 5);
        std::fs::write(cache.path(), "{not json").unwrap();
        assert!(cache.read_json().is_none());
    }
}
