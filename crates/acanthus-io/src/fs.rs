use std::io::Write;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};

use crate::error::IoError;

static SEQUENCE: AtomicU64 = AtomicU64::new(0);

/// Writes `bytes` to a temporary file next to `path`, syncs it and renames it over `path`, creating missing parents. Readers never see a partial file.
pub fn atomic_write(path: &Path, bytes: &[u8]) -> Result<(), IoError> {
    let dir = match path.parent() {
        Some(p) if !p.as_os_str().is_empty() => p,
        _ => Path::new("."),
    };
    std::fs::create_dir_all(dir)?;
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let tmp = dir.join(format!(".{name}.{}.{}.tmp", std::process::id(), SEQUENCE.fetch_add(1, Ordering::Relaxed)));
    let result = (|| -> std::io::Result<()> {
        let mut file = std::fs::File::create(&tmp)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o644))?;
        std::fs::rename(&tmp, path)
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    result.map_err(IoError::from)
}

/// Reads a file as UTF-8, replacing invalid sequences.
pub fn read_to_string_lossy(path: &Path) -> Result<String, IoError> {
    Ok(String::from_utf8_lossy(&std::fs::read(path)?).into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn writes_new_file_and_creates_parents() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("a/b/settings.json");
        atomic_write(&target, b"{}").unwrap();
        assert_eq!(std::fs::read(&target).unwrap(), b"{}");
        assert_eq!(std::fs::metadata(&target).unwrap().permissions().mode() & 0o777, 0o644);
    }

    #[test]
    fn replaces_existing_file_without_leaving_temporaries() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("f.json");
        atomic_write(&target, b"one").unwrap();
        atomic_write(&target, b"two").unwrap();
        assert_eq!(std::fs::read(&target).unwrap(), b"two");
        let entries: Vec<_> = std::fs::read_dir(dir.path()).unwrap().collect();
        assert_eq!(entries.len(), 1);
    }

    #[test]
    fn lossy_read_tolerates_invalid_utf8() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("x");
        std::fs::write(&target, [0x68, 0xff, 0x69]).unwrap();
        assert_eq!(read_to_string_lossy(&target).unwrap(), "h\u{fffd}i");
    }
}
