use std::path::PathBuf;
use std::process::Command;

use acanthus_color::generate::{SchemeType, hex};
use acanthus_runtime::{ColorMode, ImageRequest, table_from_image};
use serde_json::Value;

fn images(dir: &str) -> Vec<PathBuf> {
    let mut files: Vec<PathBuf> = std::fs::read_dir(dir)
        .unwrap()
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            p.extension()
                .and_then(|e| e.to_str())
                .is_some_and(|e| matches!(e.to_lowercase().as_str(), "png" | "jpg" | "jpeg" | "webp"))
        })
        .collect();
    files.sort();
    files
}

fn reference(path: &PathBuf, mode: &str, scheme: &str) -> Value {
    let output = Command::new("matugen")
        .args(["image"])
        .arg(path)
        .args(["--json", "hex", "-q", "--source-color-index", "0", "-m", mode, "-t", scheme])
        .output()
        .expect("matugen must be installed for this test");
    serde_json::from_slice(&output.stdout).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

#[test]
fn native_image_pipeline_matches_installed_matugen() {
    let Ok(dir) = std::env::var("SERP_MATUGEN_WALLPAPERS") else { return };
    let limit: usize = std::env::var("SERP_MATUGEN_LIMIT").ok().and_then(|v| v.parse().ok()).unwrap_or(usize::MAX);
    let combos = [
        ("dark", "scheme-tonal-spot"),
        ("smart", "scheme-smart"),
        ("light", "scheme-content"),
        ("dark", "scheme-vibrant"),
    ];
    let mut checked = 0;
    for path in images(&dir).into_iter().take(limit) {
        for (index, (mode, scheme)) in combos.iter().enumerate() {
            if index > 1 && checked % 5 != 0 {
                continue;
            }
            let expected = reference(&path, mode, scheme);
            let table = table_from_image(&ImageRequest {
                path: path.clone(),
                mode: ColorMode::parse(mode).unwrap(),
                scheme: SchemeType::parse(scheme).unwrap(),
                contrast: None,
                source_index: 0,
            })
            .unwrap();
            let want_mode = expected["mode"].as_str().unwrap();
            assert_eq!(if table.dark_default { "dark" } else { "light" }, want_mode, "{} mode", path.display());
            let colors = expected["colors"].as_object().unwrap();
            for (role, entry) in colors {
                let mine = table.roles.get(role).unwrap_or_else(|| panic!("{} missing role {role}", path.display()));
                assert_eq!(
                    hex(mine.dark),
                    entry["dark"]["color"].as_str().unwrap(),
                    "{} {mode} {scheme} {role} dark",
                    path.display()
                );
                assert_eq!(
                    hex(mine.light),
                    entry["light"]["color"].as_str().unwrap(),
                    "{} {mode} {scheme} {role} light",
                    path.display()
                );
            }
            checked += 1;
        }
    }
    assert!(checked > 0);
    eprintln!("compared {checked} image/scheme combinations");
}

#[test]
fn rendered_templates_match_installed_matugen_byte_for_byte() {
    let Ok(dir) = std::env::var("SERP_MATUGEN_WALLPAPERS") else { return };
    let image = images(&dir).into_iter().next().expect("a wallpaper");
    let assets = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/matugen");
    let work = std::env::temp_dir().join(format!("serp-matugen-oracle-{}", std::process::id()));
    let (reference_out, native_out) = (work.join("reference"), work.join("native"));
    std::fs::create_dir_all(&reference_out).unwrap();
    std::fs::create_dir_all(&native_out).unwrap();

    let mut config = String::from("[config]\nreload_apps = false\n");
    let mut specs = String::from("[config]\n");
    for entry in std::fs::read_dir(assets.join("templates")).unwrap().flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        let stem = name.trim_end_matches(".template").to_owned();
        config.push_str(&format!(
            "[templates.{stem}]\ninput_path = \"{}\"\noutput_path = \"{}\"\n",
            entry.path().display(),
            reference_out.join(&stem).display()
        ));
        specs.push_str(&format!(
            "[templates.{stem}]\ninput_path = \"{}\"\noutput_path = \"{}\"\n",
            entry.path().display(),
            native_out.join(&stem).display()
        ));
    }
    let config_path = work.join("config.toml");
    std::fs::write(&config_path, config).unwrap();
    let status = Command::new("matugen")
        .args(["image"])
        .arg(&image)
        .args(["-c"])
        .arg(&config_path)
        .args(["-q", "-m", "dark", "-t", "scheme-tonal-spot", "--source-color-index", "0"])
        .output()
        .unwrap();
    assert!(status.status.success(), "{}", String::from_utf8_lossy(&status.stderr));

    let table = table_from_image(&ImageRequest {
        path: image,
        mode: ColorMode::Dark,
        scheme: SchemeType::TonalSpot,
        contrast: None,
        source_index: 0,
    })
    .unwrap();
    let parsed = acanthus_runtime::parse_template_config(&specs, &work, &work).unwrap();
    acanthus_runtime::apply_templates(&parsed, &table).unwrap();

    let mut compared = 0;
    for entry in std::fs::read_dir(&reference_out).unwrap().flatten() {
        let mine = std::fs::read(native_out.join(entry.file_name())).unwrap();
        let theirs = std::fs::read(entry.path()).unwrap();
        assert_eq!(
            String::from_utf8_lossy(&mine),
            String::from_utf8_lossy(&theirs),
            "{}",
            entry.file_name().to_string_lossy()
        );
        compared += 1;
    }
    assert!(compared >= 4);
    let _ = std::fs::remove_dir_all(&work);
}
