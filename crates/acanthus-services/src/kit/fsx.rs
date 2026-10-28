use std::path::Path;

/// Creates `dir` and its parents; failures are logged under the name `service`.
pub fn create_dir(service: &str, dir: &Path) {
    if let Err(error) = std::fs::create_dir_all(dir) {
        tracing::warn!("{service}: cannot create {}: {error}", dir.display());
    }
}

/// Writes `bytes` to `path`, creating parent directories; failures are logged.
pub fn write_file(service: &str, path: &Path, bytes: &[u8]) {
    if let Some(parent) = path.parent().filter(|parent| !parent.as_os_str().is_empty()) {
        create_dir(service, parent);
    }
    if let Err(error) = std::fs::write(path, bytes) {
        tracing::warn!("{service}: cannot write {}: {error}", path.display());
    }
}

/// Writes `bytes` to `path` atomically, so readers never see a partial file; failures are logged.
pub fn write_atomic(service: &str, path: &Path, bytes: &[u8]) {
    if let Err(error) = acanthus_io::atomic_write(path, bytes) {
        tracing::warn!("{service}: cannot write {}: {error}", path.display());
    }
}

/// Removes `path`; a missing file is not an error and other failures are logged.
pub fn remove_file(service: &str, path: &Path) {
    match std::fs::remove_file(path) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => tracing::warn!("{service}: cannot remove {}: {error}", path.display()),
    }
}

/// Moves `from` to `to` and reports whether it worked; failures are logged.
pub fn rename(service: &str, from: &Path, to: &Path) -> bool {
    match std::fs::rename(from, to) {
        Ok(()) => true,
        Err(error) => {
            tracing::warn!("{service}: cannot move {} to {}: {error}", from.display(), to.display());
            false
        }
    }
}

/// Sets the modification time of `path` to now; a missing file is ignored and other failures are logged.
pub fn touch(service: &str, path: &Path) {
    match std::fs::OpenOptions::new().write(true).open(path) {
        Ok(file) => {
            if let Err(error) = file.set_modified(std::time::SystemTime::now()) {
                tracing::warn!("{service}: cannot touch {}: {error}", path.display());
            }
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => tracing::warn!("{service}: cannot open {}: {error}", path.display()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn writes_create_parents_and_removing_a_missing_file_is_quiet() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a").join("b.txt");
        write_file("test", &path, b"x");
        assert_eq!(std::fs::read(&path).unwrap(), b"x");
        write_atomic("test", &path, b"y");
        assert_eq!(std::fs::read(&path).unwrap(), b"y");
        remove_file("test", &path);
        remove_file("test", &path);
        assert!(!path.exists());
        touch("test", &path);
        assert!(!path.exists());
    }

    #[test]
    fn rename_reports_failure() {
        let dir = tempfile::tempdir().unwrap();
        assert!(!rename("test", &dir.path().join("missing"), &dir.path().join("to")));
        std::fs::write(dir.path().join("from"), b"1").unwrap();
        assert!(rename("test", &dir.path().join("from"), &dir.path().join("to")));
    }
}
