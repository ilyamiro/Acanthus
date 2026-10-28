use std::fs;
use std::path::{Path, PathBuf};

const FILES: [(&str, &str); 6] = [
    ("mod.rs", include_str!("templates/mod.rs.tpl")),
    ("model.rs", include_str!("templates/model.rs.tpl")),
    ("controller.rs", include_str!("templates/controller.rs.tpl")),
    ("host.rs", include_str!("templates/host.rs.tpl")),
    ("view.rs", include_str!("templates/view.rs.tpl")),
    ("tests.rs", include_str!("templates/tests.rs.tpl")),
];

pub fn scaffold(name: &str, parent: &Path) -> Result<Vec<PathBuf>, String> {
    validate(name)?;
    let dir = parent.join(name);
    if dir.exists() {
        return Err(format!("{} already exists", dir.display()));
    }
    fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let pascal = pascal_case(name);
    let mut created = Vec::new();
    for (file, template) in FILES {
        let path = dir.join(file);
        let body = template.replace("{{name}}", name).replace("{{Name}}", &pascal);
        fs::write(&path, body).map_err(|e| format!("{}: {e}", path.display()))?;
        created.push(path);
    }
    Ok(created)
}

pub fn report(name: &str, created: &[PathBuf]) -> String {
    let mut lines: Vec<String> = created.iter().map(|path| format!("created {}", path.display())).collect();
    lines.push(format!("next: add `pub mod {name};` to the parent module and a `Module` impl that builds the view"));
    lines.join("\n")
}

fn validate(name: &str) -> Result<(), String> {
    let mut chars = name.chars();
    let starts_ok = chars.next().is_some_and(|c| c.is_ascii_lowercase());
    if starts_ok && name.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_') {
        Ok(())
    } else {
        Err(format!("`{name}` is not a snake_case module name"))
    }
}

fn pascal_case(name: &str) -> String {
    name.split('_')
        .filter(|part| !part.is_empty())
        .map(|part| {
            let mut chars = part.chars();
            chars.next().map(|first| first.to_ascii_uppercase().to_string() + chars.as_str()).unwrap_or_default()
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pascal_case_joins_parts() {
        assert_eq!(pascal_case("power_menu"), "PowerMenu");
        assert_eq!(pascal_case("osd"), "Osd");
    }

    #[test]
    fn rejects_bad_names() {
        assert!(validate("Power").is_err());
        assert!(validate("1abc").is_err());
        assert!(validate("a-b").is_err());
        assert!(validate("power_menu2").is_ok());
    }

    #[test]
    fn writes_the_five_files_plus_mod_and_fills_names() {
        let tmp = tempfile::tempdir().unwrap();
        let created = scaffold("power_menu", tmp.path()).unwrap();
        assert_eq!(created.len(), 6);
        let host = fs::read_to_string(tmp.path().join("power_menu/host.rs")).unwrap();
        assert!(host.contains("PowerMenuHost"));
        assert!(!host.contains("{{"));
        assert!(scaffold("power_menu", tmp.path()).is_err());
    }

    #[test]
    fn templates_carry_no_comments() {
        for (_, template) in FILES {
            assert!(!template.lines().any(|line| line.trim_start().starts_with("//")));
        }
    }
}
