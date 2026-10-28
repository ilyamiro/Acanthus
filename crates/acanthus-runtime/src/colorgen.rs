use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use acanthus_color::color::Argb;
use acanthus_color::generate::{
    IMAGE_SAMPLE_SIZE, SMART_THUMBNAIL_SIZE, SchemeType, colorfulness, detect_variant, hex, is_light_average,
    ranked_source_colors, roles, scheme,
};
use acanthus_io::atomic_write;
use image::imageops::FilterType;
use image::{DynamicImage, GenericImageView, ImageReader};
use serde_json::Value;

#[derive(Debug, thiserror::Error)]
pub enum ColorGenError {
    #[error("cannot read image {0}: {1}")]
    Image(PathBuf, String),
    #[error("image has no usable colours")]
    NoColors,
    #[error("source colour index {0} is out of range")]
    Index(usize),
    #[error("invalid colour {0}")]
    Color(String),
    #[error("template config: {0}")]
    Config(String),
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Write(#[from] acanthus_io::IoError),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Dark,
    Light,
    Smart,
}

impl Mode {
    pub fn parse(text: &str) -> Option<Mode> {
        match text.trim().to_lowercase().as_str() {
            "dark" => Some(Mode::Dark),
            "light" => Some(Mode::Light),
            "smart" => Some(Mode::Smart),
            _ => None,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Mode::Dark => "dark",
            Mode::Light => "light",
            Mode::Smart => "smart",
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct ModeColors {
    pub dark: Argb,
    pub light: Argb,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ColorTable {
    pub roles: BTreeMap<String, ModeColors>,
    pub dark_default: bool,
    pub image: Option<String>,
}

impl ColorTable {
    pub fn default_for(&self, role: &str) -> Option<Argb> {
        let colors = self.roles.get(role)?;
        Some(if self.dark_default { colors.dark } else { colors.light })
    }

    pub fn from_schemes(
        dark: Vec<(String, Argb)>,
        light: Vec<(String, Argb)>,
        source: Argb,
        dark_default: bool,
    ) -> ColorTable {
        let mut roles: BTreeMap<String, ModeColors> = BTreeMap::new();
        for (name, color) in dark {
            roles.insert(name, ModeColors { dark: color, light: color });
        }
        for (name, color) in light {
            if let Some(entry) = roles.get_mut(&name) {
                entry.light = color;
            }
        }
        roles.insert("source_color".to_owned(), ModeColors { dark: source, light: source });
        ColorTable { roles, dark_default, image: None }
    }

    pub fn from_json(value: &Value, dark_default: bool) -> Result<ColorTable, ColorGenError> {
        let colors = value
            .get("colors")
            .and_then(Value::as_object)
            .ok_or_else(|| ColorGenError::Color("missing colors".into()))?;
        let mut roles = BTreeMap::new();
        for (name, entry) in colors {
            let pick = |mode: &str| -> Result<Argb, ColorGenError> {
                let text = entry
                    .get(mode)
                    .or_else(|| entry.get("default"))
                    .and_then(|m| m.get("hex").or_else(|| m.get("color")))
                    .and_then(Value::as_str)
                    .ok_or_else(|| ColorGenError::Color(format!("{name}.{mode}")))?;
                text.parse::<Argb>().map_err(|_| ColorGenError::Color(text.to_owned()))
            };
            roles.insert(name.clone(), ModeColors { dark: pick("dark")?, light: pick("light")? });
        }
        Ok(ColorTable { roles, dark_default, image: None })
    }
}

pub fn decode_image(path: &Path) -> Result<DynamicImage, ColorGenError> {
    let reader = ImageReader::open(path).map_err(|e| ColorGenError::Image(path.to_path_buf(), e.to_string()))?;
    reader
        .with_guessed_format()
        .map_err(|e| ColorGenError::Image(path.to_path_buf(), e.to_string()))?
        .decode()
        .map_err(|e| ColorGenError::Image(path.to_path_buf(), e.to_string()))
}

pub fn sample_pixels(image: &DynamicImage) -> Vec<Argb> {
    let rgba = image.to_rgba8();
    let resized = image::imageops::resize(&rgba, IMAGE_SAMPLE_SIZE, IMAGE_SAMPLE_SIZE, FilterType::Triangle);
    resized.pixels().map(|p| Argb::new(p.0[3], p.0[0], p.0[1], p.0[2])).collect()
}

pub fn source_colors(image: &DynamicImage) -> Vec<Argb> {
    ranked_source_colors(&sample_pixels(image))
}

pub fn smart_options(image: &DynamicImage) -> (bool, SchemeType) {
    let thumbnail = image.thumbnail(SMART_THUMBNAIL_SIZE, SMART_THUMBNAIL_SIZE);
    let average = thumbnail.resize_exact(1, 1, FilterType::Lanczos3).get_pixel(0, 0).0;
    let light = is_light_average([average[0], average[1], average[2]]);
    let rgb: Vec<[u8; 3]> = thumbnail.to_rgb8().pixels().map(|p| p.0).collect();
    (!light, detect_variant(colorfulness(&rgb)))
}

#[derive(Debug, Clone)]
pub struct ImageRequest {
    pub path: PathBuf,
    pub mode: Mode,
    pub scheme: SchemeType,
    pub contrast: Option<f64>,
    pub source_index: usize,
}

pub fn table_from_image(request: &ImageRequest) -> Result<ColorTable, ColorGenError> {
    let image = decode_image(&request.path)?;
    let needs_smart = request.mode == Mode::Smart || request.scheme == SchemeType::Smart;
    let smart = needs_smart.then(|| smart_options(&image));
    let dark_default = match request.mode {
        Mode::Dark => true,
        Mode::Light => false,
        Mode::Smart => smart.is_none_or(|(dark, _)| dark),
    };
    let kind = match request.scheme {
        SchemeType::Smart => smart.map_or(SchemeType::TonalSpot, |(_, kind)| kind),
        other => other,
    };
    let ranked = source_colors(&image);
    let source = *ranked.get(request.source_index).ok_or(ColorGenError::Index(request.source_index))?;
    let mut table = table_from_source(source, kind, request.contrast, dark_default);
    table.image = std::fs::canonicalize(&request.path).ok().map(|p| p.to_string_lossy().into_owned());
    Ok(table)
}

pub fn table_from_source(source: Argb, kind: SchemeType, contrast: Option<f64>, dark_default: bool) -> ColorTable {
    ColorTable::from_schemes(
        roles(scheme(source, kind, true, contrast)),
        roles(scheme(source, kind, false, contrast)),
        source,
        dark_default,
    )
}

fn property(color: Argb, name: &str) -> Option<String> {
    Some(match name {
        "hex" | "color" => hex(color),
        "hex_stripped" => hex(color)[1..].to_owned(),
        "rgb" => format!("rgb({}, {}, {})", color.red, color.green, color.blue),
        "red" => color.red.to_string(),
        "green" => color.green.to_string(),
        "blue" => color.blue.to_string(),
        _ => return None,
    })
}

fn resolve(table: &ColorTable, path: &str) -> Option<String> {
    let parts: Vec<&str> = path.split('.').map(str::trim).collect();
    match parts.as_slice() {
        ["mode"] => Some(if table.dark_default { "dark" } else { "light" }.to_owned()),
        ["is_dark_mode"] => Some(table.dark_default.to_string()),
        ["image"] => table.image.clone(),
        ["colors", role, mode, prop] => {
            let colors = table.roles.get(*role)?;
            let color = match *mode {
                "dark" => colors.dark,
                "light" => colors.light,
                "default" => {
                    if table.dark_default {
                        colors.dark
                    } else {
                        colors.light
                    }
                }
                _ => return None,
            };
            property(color, prop)
        }
        _ => None,
    }
}

pub fn render_template(text: &str, table: &ColorTable) -> (String, Vec<String>) {
    let mut out = String::with_capacity(text.len());
    let mut unresolved = Vec::new();
    let mut rest = text;
    while let Some(start) = rest.find("{{") {
        out.push_str(&rest[..start]);
        let after = &rest[start + 2..];
        match after.find("}}") {
            Some(end) => {
                let expression = after[..end].trim();
                match resolve(table, expression) {
                    Some(value) => out.push_str(&value),
                    None => {
                        unresolved.push(expression.to_owned());
                        out.push_str(&rest[start..start + 2 + end + 2]);
                    }
                }
                rest = &after[end + 2..];
            }
            None => {
                out.push_str(&rest[start..]);
                rest = "";
            }
        }
    }
    out.push_str(rest);
    (out, unresolved)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TemplateSpec {
    pub name: String,
    pub input: PathBuf,
    pub output: PathBuf,
    pub enabled: bool,
}

pub fn expand_home(path: &str, home: &Path) -> PathBuf {
    match path.strip_prefix("~/") {
        Some(rest) => home.join(rest),
        None if path == "~" => home.to_path_buf(),
        None => PathBuf::from(path),
    }
}

pub fn parse_template_config(text: &str, config_dir: &Path, home: &Path) -> Result<Vec<TemplateSpec>, ColorGenError> {
    let mut specs: Vec<TemplateSpec> = Vec::new();
    let mut current: Option<usize> = None;
    for (number, raw) in text.lines().enumerate() {
        let line = raw.split('#').next().unwrap_or("").trim();
        if line.is_empty() {
            continue;
        }
        if let Some(header) = line.strip_prefix('[').and_then(|l| l.strip_suffix(']')) {
            current = header.strip_prefix("templates.").map(|name| {
                specs.push(TemplateSpec {
                    name: name.to_owned(),
                    input: PathBuf::new(),
                    output: PathBuf::new(),
                    enabled: true,
                });
                specs.len() - 1
            });
            continue;
        }
        let (Some(index), Some((key, value))) = (current, line.split_once('=')) else { continue };
        let value = value.trim();
        let unquoted = value.strip_prefix('"').and_then(|v| v.strip_suffix('"'));
        match key.trim() {
            "input_path" => {
                let path = unquoted.ok_or_else(|| {
                    ColorGenError::Config(format!("line {}: input_path must be a string", number + 1))
                })?;
                specs[index].input = config_dir.join(path);
            }
            "output_path" => {
                let path = unquoted.ok_or_else(|| {
                    ColorGenError::Config(format!("line {}: output_path must be a string", number + 1))
                })?;
                specs[index].output = expand_home(path, home);
            }
            "enabled" => specs[index].enabled = value != "false",
            _ => {}
        }
    }
    specs.retain(|s| !s.input.as_os_str().is_empty() && !s.output.as_os_str().is_empty());
    Ok(specs)
}

pub fn apply_templates(specs: &[TemplateSpec], table: &ColorTable) -> Result<Vec<PathBuf>, ColorGenError> {
    let mut written = Vec::new();
    for spec in specs.iter().filter(|s| s.enabled) {
        let Ok(bytes) = std::fs::read(&spec.input) else {
            tracing::warn!("template {} missing at {}", spec.name, spec.input.display());
            continue;
        };
        let (rendered, unresolved) = render_template(&String::from_utf8_lossy(&bytes), table);
        for expression in unresolved {
            tracing::warn!("template {}: cannot resolve {{{{{expression}}}}}", spec.name);
        }
        atomic_write(&spec.output, rendered.as_bytes())?;
        written.push(spec.output.clone());
    }
    Ok(written)
}

pub fn signal_processes_named(names: &[&str], signal: rustix::process::Signal) -> usize {
    let Ok(entries) = std::fs::read_dir("/proc") else { return 0 };
    let mut sent = 0;
    for entry in entries.flatten() {
        let Some(pid) = entry.file_name().to_str().and_then(|n| n.parse::<i32>().ok()) else { continue };
        let Ok(comm) = std::fs::read_to_string(entry.path().join("comm")) else { continue };
        if names.contains(&comm.trim())
            && let Some(pid) = rustix::process::Pid::from_raw(pid)
            && rustix::process::kill_process(pid, signal).is_ok()
        {
            sent += 1;
        }
    }
    sent
}

#[cfg(test)]
mod tests {
    use acanthus_color::generate::SchemeType;
    use serde_json::json;

    use super::*;
    use crate::to_md3;

    fn table() -> ColorTable {
        table_from_source(Argb::new(255, 255, 0, 0), SchemeType::TonalSpot, None, true)
    }

    #[test]
    fn templates_substitute_roles_modes_and_properties() {
        let t = table();
        let (out, unresolved) = render_template(
            "a={{colors.primary.default.hex}} b={{ colors.primary.light.hex }} c={{colors.primary.dark.hex_stripped}} m={{mode}} d={{is_dark_mode}} r={{colors.source_color.default.rgb}}",
            &t,
        );
        assert_eq!(out, "a=#ffb4a8 b=#904b40 c=ffb4a8 m=dark d=true r=rgb(255, 0, 0)");
        assert!(unresolved.is_empty());

        let light = table_from_source(Argb::new(255, 255, 0, 0), SchemeType::TonalSpot, None, false);
        assert_eq!(render_template("{{colors.primary.default.hex}}", &light).0, "#904b40");
    }

    #[test]
    fn unknown_expressions_stay_visible_and_are_reported() {
        let (out, unresolved) =
            render_template("x {{colors.nope.default.hex}} y {{colors.primary.default.bogus}} z {{open", &table());
        assert_eq!(out, "x {{colors.nope.default.hex}} y {{colors.primary.default.bogus}} z {{open");
        assert_eq!(unresolved, ["colors.nope.default.hex", "colors.primary.default.bogus"]);
    }

    #[test]
    fn template_config_parses_paths_relative_to_the_config_and_home() {
        let text = "[config]\nreload_apps = false\n\n[templates.kitty]\ninput_path = \"templates/k.conf\"\noutput_path = \"~/.config/kitty/colors.conf\"\n\n[templates.off]\ninput_path = \"x\"\noutput_path = \"/tmp/y\"\nenabled = false\n";
        let specs = parse_template_config(text, Path::new("/assets/matugen"), Path::new("/home/u")).unwrap();
        assert_eq!(specs.len(), 2);
        assert_eq!(specs[0].input, PathBuf::from("/assets/matugen/templates/k.conf"));
        assert_eq!(specs[0].output, PathBuf::from("/home/u/.config/kitty/colors.conf"));
        assert!(!specs[1].enabled);
    }

    #[test]
    fn bundled_template_configs_parse_and_reference_existing_templates() {
        let paths = crate::paths::test_paths();
        let dir = paths.assets.join("matugen");
        for name in ["config.toml", "config-static.toml"] {
            let text = std::fs::read_to_string(dir.join(name)).unwrap();
            let specs = parse_template_config(&text, &dir, Path::new("/home/u")).unwrap();
            assert!(specs.len() >= 3, "{name}");
            for spec in specs {
                assert!(spec.input.is_file(), "{}", spec.input.display());
            }
        }
    }

    #[test]
    fn applying_templates_writes_atomically_and_creates_directories() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("in.tpl"), "bg {{colors.surface.default.hex}}\n").unwrap();
        let spec = TemplateSpec {
            name: "t".into(),
            input: dir.path().join("in.tpl"),
            output: dir.path().join("deep/out.conf"),
            enabled: true,
        };
        let written = apply_templates(&[spec], &table()).unwrap();
        assert_eq!(written.len(), 1);
        assert!(std::fs::read_to_string(dir.path().join("deep/out.conf")).unwrap().starts_with("bg #"));
    }

    #[test]
    fn md3_json_round_trips_into_a_color_table() {
        let md3 = to_md3(&json!({"base": "#112233", "crust": "#000000", "text": "#ffffff"}));
        let table = ColorTable::from_json(&md3, true).unwrap();
        assert_eq!(hex(table.default_for("surface_container_lowest").unwrap()), "#112233");
        assert!(ColorTable::from_json(&json!({}), true).is_err());
    }

    fn write_png(path: &Path, rgba: [u8; 4], size: u32) {
        let image = image::RgbaImage::from_pixel(size, size, image::Rgba(rgba));
        image.save(path).unwrap();
    }

    #[test]
    fn image_pipeline_picks_the_dominant_colour_and_honours_mode() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("wall.png");
        write_png(&path, [200, 30, 60, 255], 64);
        let request = ImageRequest {
            path: path.clone(),
            mode: Mode::Light,
            scheme: SchemeType::TonalSpot,
            contrast: None,
            source_index: 0,
        };
        let table = table_from_image(&request).unwrap();
        assert!(!table.dark_default);
        assert_eq!(hex(table.roles["source_color"].dark), "#c81e3c");
        assert!(table.image.is_some());
        assert!(matches!(
            table_from_image(&ImageRequest { source_index: 9, ..request.clone() }),
            Err(ColorGenError::Index(9))
        ));
        assert!(matches!(
            table_from_image(&ImageRequest { path: dir.path().join("missing.png"), ..request }),
            Err(ColorGenError::Image(..))
        ));
    }

    #[test]
    fn smart_mode_follows_the_image_tone_and_colourfulness() {
        let dir = tempfile::tempdir().unwrap();
        let dark = dir.path().join("dark.png");
        let light = dir.path().join("light.png");
        write_png(&dark, [20, 20, 30, 255], 64);
        write_png(&light, [240, 240, 250, 255], 64);
        let smart = |path: &Path| {
            table_from_image(&ImageRequest {
                path: path.to_path_buf(),
                mode: Mode::Smart,
                scheme: SchemeType::Smart,
                contrast: None,
                source_index: 0,
            })
            .unwrap()
        };
        assert!(smart(&dark).dark_default);
        assert!(!smart(&light).dark_default);
    }

    #[test]
    fn signalling_processes_by_name_reaches_a_real_process() {
        let mut child = std::process::Command::new("sleep").arg("30").spawn().unwrap();
        let sent = signal_processes_named(&["sleep"], rustix::process::Signal::USR1);
        assert!(sent >= 1);
        let status = child.wait().unwrap();
        assert!(!status.success());
        assert_eq!(signal_processes_named(&["definitely-not-a-process"], rustix::process::Signal::USR1), 0);
    }
}
