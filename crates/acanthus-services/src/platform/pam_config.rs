use std::fs;
use std::io;
use std::path::{Path, PathBuf};

pub fn no_fingerprint_config(service: &str) -> String {
    format!("{service}-nofp")
}

const FALLBACK_SERVICE: &str = "login";
const FINGERPRINT_MODULE: &str = "pam_fprintd";
const FINGERPRINT_TOOLS: [&str; 2] = ["fprintd-list", "fprintd-enroll"];
const MAX_INCLUDE_DEPTH: usize = 5;

fn strip_module(text: &str) -> String {
    let mut kept: Vec<&str> = text.lines().filter(|line| !line.contains(FINGERPRINT_MODULE)).collect();
    if !kept.is_empty() {
        kept.push("");
    }
    kept.join("\n")
}

fn include_target(line: &str) -> Option<String> {
    let line = line.split('#').next().unwrap_or("");
    let words: Vec<&str> = line.split_whitespace().collect();
    match words.first().copied() {
        Some("auth" | "-auth") => match words.get(1).copied() {
            Some("include" | "substack") => words.get(2).map(|target| (*target).to_owned()),
            _ => None,
        },
        Some("@include") => words.get(1).map(|target| (*target).to_owned()),
        _ => None,
    }
}

fn mentions_fingerprint(file: &Path, directory: &Path, depth: usize) -> bool {
    if depth > MAX_INCLUDE_DEPTH {
        return false;
    }
    let Ok(text) = fs::read_to_string(file) else { return false };
    for line in text.lines() {
        let code = line.split('#').next().unwrap_or("");
        if code.contains(FINGERPRINT_MODULE) {
            return true;
        }
        let Some(target) = include_target(line) else { continue };
        let target = if target.starts_with('/') { PathBuf::from(target) } else { directory.join(target) };
        if mentions_fingerprint(&target, directory, depth + 1) {
            return true;
        }
    }
    false
}

fn base_config(directory: &Path, service: &str) -> PathBuf {
    let base = directory.join(service);
    if base.exists() { base } else { directory.join(FALLBACK_SERVICE) }
}

fn tool_on_path(path_dirs: &[PathBuf], name: &str) -> bool {
    path_dirs.iter().any(|dir| dir.join(name).is_file())
}

pub fn fingerprint_available_in(directory: &Path, path_dirs: &[PathBuf], service: &str) -> bool {
    if !FINGERPRINT_TOOLS.iter().any(|tool| tool_on_path(path_dirs, tool)) {
        return false;
    }
    mentions_fingerprint(&base_config(directory, service), directory, 0)
}

pub fn fingerprint_available(directory: &Path, service: &str) -> bool {
    let path_dirs: Vec<PathBuf> =
        std::env::var_os("PATH").map(|path| std::env::split_paths(&path).collect()).unwrap_or_default();
    fingerprint_available_in(directory, &path_dirs, service)
}

pub fn build_no_fingerprint_dir(source: &Path, destination: &Path, service: &str) -> io::Result<()> {
    if destination.as_os_str().is_empty() || !source.is_dir() {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "pam directories missing"));
    }
    let base = base_config(source, service);
    let base_text = fs::read_to_string(&base)?;
    if destination.exists() {
        fs::remove_dir_all(destination)?;
    }
    fs::create_dir_all(destination)?;
    for entry in fs::read_dir(source)? {
        let path = entry?.path();
        if !path.is_file() {
            continue;
        }
        let Some(name) = path.file_name() else { continue };
        let target = destination.join(name);
        match fs::read_to_string(&path) {
            Ok(text) if text.contains(FINGERPRINT_MODULE) => fs::write(&target, strip_module(&text))?,
            _ => {
                let resolved = fs::canonicalize(&path)?;
                crate::kit::fsx::remove_file("pam", &target);
                std::os::unix::fs::symlink(resolved, &target)?;
            }
        }
    }
    let target = destination.join(no_fingerprint_config(service));
    crate::kit::fsx::remove_file("pam", &target);
    if base_text.contains(FINGERPRINT_MODULE) {
        fs::write(&target, strip_module(&base_text))?;
    } else {
        fs::write(&target, base_text)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const SERVICE: &str = "demo";

    fn write(dir: &Path, name: &str, text: &str) {
        fs::write(dir.join(name), text).unwrap();
    }

    fn tools(dir: &Path, name: &str) -> Vec<PathBuf> {
        let bin = dir.join("bin");
        fs::create_dir_all(&bin).unwrap();
        fs::write(bin.join(name), "").unwrap();
        vec![bin]
    }

    #[test]
    fn detects_fprintd_directly_and_through_includes() {
        let dir = tempfile::tempdir().unwrap();
        let path = tools(dir.path(), "fprintd-list");
        let pam = dir.path().join("pam.d");
        fs::create_dir_all(&pam).unwrap();
        write(&pam, SERVICE, "auth include common\naccount required pam_unix.so\n");
        write(&pam, "common", "auth sufficient pam_fprintd.so\n");
        assert!(fingerprint_available_in(&pam, &path, SERVICE));
        write(&pam, "common", "# auth sufficient pam_fprintd.so\nauth required pam_unix.so\n");
        assert!(!fingerprint_available_in(&pam, &path, SERVICE));
        write(&pam, SERVICE, "auth sufficient /x/pam_fprintd.so # fprintd\n");
        assert!(fingerprint_available_in(&pam, &path, SERVICE));
    }

    #[test]
    fn needs_a_fprintd_tool_and_falls_back_to_login() {
        let dir = tempfile::tempdir().unwrap();
        let pam = dir.path().join("pam.d");
        fs::create_dir_all(&pam).unwrap();
        write(&pam, "login", "@include shared\n");
        write(&pam, "shared", "auth sufficient pam_fprintd.so\n");
        assert!(!fingerprint_available_in(&pam, &[], SERVICE));
        assert!(fingerprint_available_in(&pam, &tools(dir.path(), "fprintd-enroll"), SERVICE));
    }

    #[test]
    fn include_loops_stop_at_the_depth_limit() {
        let dir = tempfile::tempdir().unwrap();
        let path = tools(dir.path(), "fprintd-list");
        let pam = dir.path().join("pam.d");
        fs::create_dir_all(&pam).unwrap();
        write(&pam, SERVICE, "auth substack demo\n");
        assert!(!fingerprint_available_in(&pam, &path, SERVICE));
    }

    #[test]
    fn no_fingerprint_directory_strips_the_module_and_links_the_rest() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("src");
        let destination = dir.path().join("dst");
        fs::create_dir_all(&source).unwrap();
        write(&source, SERVICE, "auth sufficient pam_fprintd.so\nauth required pam_unix.so\n");
        write(&source, "other", "auth required pam_deny.so\n");
        write(&source, "common", "auth sufficient pam_fprintd.so\naccount required pam_permit.so\n");
        build_no_fingerprint_dir(&source, &destination, SERVICE).unwrap();
        assert_eq!(
            fs::read_to_string(destination.join(no_fingerprint_config(SERVICE))).unwrap(),
            "auth required pam_unix.so\n"
        );
        assert_eq!(fs::read_to_string(destination.join("common")).unwrap(), "account required pam_permit.so\n");
        assert!(destination.join("other").symlink_metadata().unwrap().file_type().is_symlink());
        assert_eq!(fs::read_to_string(destination.join("other")).unwrap(), "auth required pam_deny.so\n");
        build_no_fingerprint_dir(&source, &destination, SERVICE).unwrap();
    }

    #[test]
    fn no_fingerprint_config_copies_a_clean_login_fallback() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("src");
        fs::create_dir_all(&source).unwrap();
        write(&source, "login", "auth required pam_unix.so\n");
        let destination = dir.path().join("dst");
        build_no_fingerprint_dir(&source, &destination, SERVICE).unwrap();
        assert_eq!(
            fs::read_to_string(destination.join(no_fingerprint_config(SERVICE))).unwrap(),
            "auth required pam_unix.so\n"
        );
        assert!(build_no_fingerprint_dir(&dir.path().join("missing"), &destination, SERVICE).is_err());
        assert!(build_no_fingerprint_dir(&source, Path::new(""), SERVICE).is_err());
        fs::remove_file(source.join("login")).unwrap();
        assert!(build_no_fingerprint_dir(&source, &destination, SERVICE).is_err());
    }
}
