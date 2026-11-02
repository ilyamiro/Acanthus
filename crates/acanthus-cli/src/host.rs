use std::io::Write;
use std::path::{Path, PathBuf};

use rustix::process::{Pid, Signal, kill_process, test_kill_process};

pub(crate) const DEFAULT_NAME: &str = "default";
const FILE: &str = "shell.acn";

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Instance {
    pub pid: u32,
    pub name: String,
    pub config: PathBuf,
}

pub(crate) fn resolve_config(config_home: &Path, spec: Option<&str>) -> Result<PathBuf, String> {
    let named = |name: &str| config_home.join("acanthus").join(name).join(FILE);
    let path = match spec {
        None => named(DEFAULT_NAME),
        Some(spec) if Path::new(spec).is_dir() => Path::new(spec).join(FILE),
        Some(spec) if Path::new(spec).is_file() => PathBuf::from(spec),
        Some(spec) => named(spec),
    };
    if path.is_file() { Ok(path) } else { Err(format!("no configuration at {}", path.display())) }
}

fn alive(pid: u32) -> bool {
    i32::try_from(pid).ok().and_then(Pid::from_raw).is_some_and(|pid| test_kill_process(pid).is_ok())
}

pub(crate) fn list(dir: &Path) -> Vec<Instance> {
    let Ok(entries) = std::fs::read_dir(dir) else { return Vec::new() };
    let mut found = Vec::new();
    for entry in entries.flatten() {
        let Some(pid) = entry.file_name().to_str().and_then(|n| n.parse::<u32>().ok()) else { continue };
        if !alive(pid) {
            let _ = std::fs::remove_file(entry.path());
            continue;
        }
        let text = std::fs::read_to_string(entry.path()).unwrap_or_default();
        let field = |key: &str| {
            text.lines()
                .find_map(|l| l.strip_prefix(key).and_then(|r| r.strip_prefix('=')))
                .unwrap_or_default()
                .to_owned()
        };
        found.push(Instance { pid, name: field("name"), config: PathBuf::from(field("config")) });
    }
    found.sort_by_key(|i| i.pid);
    found
}

pub(crate) fn list_command(dir: &Path, out: &mut dyn Write) {
    for instance in list(dir) {
        let _ = writeln!(out, "{}\t{}\t{}", instance.pid, instance.name, instance.config.display());
    }
}

pub(crate) fn kill_command(dir: &Path, args: &[String], out: &mut dyn Write) -> Result<i32, String> {
    let (flag, value) = match args {
        [flag, value] if flag == "--pid" || flag == "-n" => (flag.as_str(), value.as_str()),
        _ => return Err("kill takes --pid PID or -n NAME".into()),
    };
    let matching: Vec<Instance> = list(dir)
        .into_iter()
        .filter(|i| if flag == "--pid" { i.pid.to_string() == value } else { i.name == value })
        .collect();
    if matching.is_empty() {
        return Err(format!("no running instance matches {value}"));
    }
    for instance in matching {
        let pid = i32::try_from(instance.pid).ok().and_then(Pid::from_raw).ok_or("bad pid")?;
        kill_process(pid, Signal::TERM).map_err(|e| format!("cannot stop {}: {e}", instance.pid))?;
        let _ = writeln!(out, "stopped {} {}", instance.pid, instance.name);
    }
    Ok(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn register(dir: &Path, instance: &Instance) -> std::io::Result<()> {
        std::fs::create_dir_all(dir)?;
        let text = format!("name={}\nconfig={}\n", instance.name, instance.config.display());
        std::fs::write(dir.join(instance.pid.to_string()), text)
    }

    fn unregister(dir: &Path, pid: u32) {
        let _ = std::fs::remove_file(dir.join(pid.to_string()));
    }

    #[test]
    fn config_resolves_by_name_directory_and_file() {
        let home = tempfile::tempdir().unwrap();
        let dir = home.path().join("acanthus").join("bar");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("shell.acn"), "acn 0\n").unwrap();
        assert_eq!(resolve_config(home.path(), Some("bar")).unwrap(), dir.join("shell.acn"));
        assert_eq!(resolve_config(home.path(), Some(dir.to_str().unwrap())).unwrap(), dir.join("shell.acn"));
        assert_eq!(
            resolve_config(home.path(), Some(dir.join("shell.acn").to_str().unwrap())).unwrap(),
            dir.join("shell.acn")
        );
        assert!(resolve_config(home.path(), None).unwrap_err().contains("default"));
    }

    #[test]
    fn the_registry_lists_live_instances_drops_dead_ones_and_kills_by_pid() {
        let dir = tempfile::tempdir().unwrap();
        let mut child = std::process::Command::new("sleep").arg("30").spawn().unwrap();
        let pid = child.id();
        let live = Instance { pid, name: "bar".into(), config: "/c/shell.acn".into() };
        register(dir.path(), &live).unwrap();
        register(dir.path(), &Instance { pid: 4_000_000, name: "gone".into(), config: "/x".into() }).unwrap();
        assert_eq!(list(dir.path()), vec![live]);
        assert!(!dir.path().join("4000000").exists());
        let mut out = Vec::new();
        assert!(kill_command(dir.path(), &["-n".into(), "nobody".into()], &mut out).is_err());
        kill_command(dir.path(), &["--pid".into(), pid.to_string()], &mut out).unwrap();
        child.wait().unwrap();
        assert!(list(dir.path()).is_empty());
        unregister(dir.path(), pid);
    }
}
