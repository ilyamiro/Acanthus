#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CacheFile {
    path: std::path::PathBuf,
}

impl CacheFile {
    pub fn new(path: std::path::PathBuf) -> CacheFile {
        CacheFile { path }
    }

    pub fn path(&self) -> &std::path::Path {
        &self.path
    }

    pub fn read_json(&self) -> Option<serde_json::Value> {
        serde_json::from_str(std::fs::read_to_string(&self.path).ok()?.trim()).ok()
    }

    pub fn write_json(&self, value: &serde_json::Value) {
        crate::kit::fsx::write_atomic("cache", &self.path, value.to_string().as_bytes());
    }

    pub fn touch(&self) {
        crate::kit::fsx::touch("cache", &self.path);
    }

    pub fn age_seconds(&self) -> Option<u64> {
        let modified = std::fs::metadata(&self.path).ok()?.modified().ok()?;
        Some(std::time::SystemTime::now().duration_since(modified).map(|d| d.as_secs()).unwrap_or(0))
    }

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
