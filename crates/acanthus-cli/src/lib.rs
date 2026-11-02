//! The `acanthus` command line. [`run`] is the whole program; `main` only forwards the arguments.
#![deny(missing_docs)]

use acanthus_app::testing::{Headless, Script};
use acanthus_markup::{Built, Diagnostic, Functions, Scope, Value, build, parse};
use acanthus_runtime::Theme;
use acanthus_scene::{Fonts, Scene};
use acanthus_widgets::standard_registry;
use std::io::Write;
use std::path::Path;
use std::rc::Rc;

const USAGE: &str = "usage:
  acanthus check [--global NAME]... FILE...   parse and compile files without showing them
  acanthus fmt [--check] FILE...              re-indent files in place
  acanthus new FILE                           write a starter file
  acanthus shot [--size WxH] [--scale S] [--script STEPS] [--time MS] [--reduced-motion] [--global NAME]... FILE OUT.png
                                              render a file offscreen to a picture
  acanthus docs [TOPIC]                       print a guide of this version (no topic lists them)
";

const GUIDES: [(&str, &str); 2] =
    [("acn-spec", include_str!("../guides/acn-spec.md")), ("versioning", include_str!("../guides/versioning.md"))];

const TEMPLATE: &str = "acn 0\nSlider {\n    id: volume\n    width: 240\n    value: 0.5\n    on_moved: log(value)\n}\n";

struct Declared(Vec<String>);

impl Scope for Declared {
    fn roots(&self) -> Vec<String> {
        self.0.clone()
    }

    fn get(&self, _: &[String]) -> Option<Value> {
        None
    }
}

/// Runs the command in `args` (without the program name), writing results to `out` and problems to `err`; returns the exit code.
pub fn run(args: &[String], out: &mut dyn Write, err: &mut dyn Write) -> i32 {
    let rest = &args[args.len().min(1)..];
    let code = match args.first().map(String::as_str) {
        Some("check") => check(rest, out, err),
        Some("fmt") => fmt(rest, out, err),
        Some("new") => new(rest, out, err),
        Some("shot") => shot(rest, out, err),
        Some("docs") => docs(rest, out),
        _ => Err(String::new()),
    };
    match code {
        Ok(code) => code,
        Err(message) => {
            if !message.is_empty() {
                let _ = writeln!(err, "{message}");
            }
            let _ = write!(err, "{USAGE}");
            2
        }
    }
}

fn check(args: &[String], out: &mut dyn Write, err: &mut dyn Write) -> Result<i32, String> {
    let (mut globals, mut files) = (vec!["log".to_string()], Vec::new());
    let mut it = args.iter();
    while let Some(a) = it.next() {
        if a == "--global" {
            globals.push(it.next().ok_or("--global needs a name")?.clone());
        } else {
            files.push(a);
        }
    }
    if files.is_empty() {
        return Err("check needs a file".into());
    }
    let mut failed = 0;
    for file in files {
        let src = std::fs::read_to_string(file).map_err(|e| format!("{file}: {e}"))?;
        let mut scene = Scene::new(Fonts::new());
        match compile(&src, &globals, &mut scene) {
            Ok(_) => {
                let _ = writeln!(out, "ok    {file}");
            }
            Err(errors) => {
                failed += 1;
                for d in errors {
                    let _ = writeln!(err, "{}", d.render(file, &src));
                }
            }
        }
    }
    Ok(i32::from(failed > 0))
}

fn compile(src: &str, globals: &[String], scene: &mut Scene) -> Result<Built, Vec<Diagnostic>> {
    let theme = Theme::new();
    let registry = standard_registry().with_colors(move |name| theme.role(name));
    let mut functions = Functions::standard();
    functions.register("log", 1..=1, |_| Ok(Value::Null));
    let doc = parse(src).map_err(|d| vec![d])?;
    let root = scene.root();
    build(&doc, &registry, Rc::new(functions), Rc::new(Declared(globals.to_vec())), scene, root)
}

fn shot(args: &[String], out: &mut dyn Write, err: &mut dyn Write) -> Result<i32, String> {
    let (mut globals, mut files) = (vec!["log".to_string()], Vec::new());
    let (mut size, mut scale, mut script, mut time) = ((480u32, 320u32), 1.0f32, String::new(), 0u32);
    let mut it = args.iter();
    while let Some(a) = it.next() {
        let mut value = |what: &str| it.next().cloned().ok_or(format!("{a} needs {what}"));
        match a.as_str() {
            "--global" => globals.push(value("a name")?),
            "--reduced-motion" => acanthus_anim::set_reduced_motion(true),
            "--script" => script = value("steps")?,
            "--scale" => scale = value("a number")?.parse().map_err(|_| "--scale needs a number")?,
            "--time" => time = value("milliseconds")?.parse().map_err(|_| "--time needs milliseconds")?,
            "--size" => {
                let v = value("WxH")?;
                size = v
                    .split_once('x')
                    .and_then(|(w, h)| Some((w.parse().ok()?, h.parse().ok()?)))
                    .ok_or("--size needs WxH")?;
            }
            _ => files.push(a),
        }
    }
    let [file, png] = files.as_slice() else { return Err("shot needs a file and a picture path".into()) };
    let script = Script::parse(&script).map_err(|e| format!("--script: {e}"))?;
    let src = std::fs::read_to_string(file).map_err(|e| format!("{file}: {e}"))?;
    let mut scene = Scene::new(Fonts::new());
    if let Err(errors) = compile(&src, &globals, &mut scene) {
        for d in errors {
            let _ = writeln!(err, "{}", d.render(file, &src));
        }
        return Ok(1);
    }
    let headless = Headless::new(size.0, size.1, scale).map_err(|e| e.to_string())?;
    let run = headless.run(&mut scene, &script, time);
    run.image.save_png(png).map_err(|e| format!("{png}: {e}"))?;
    let _ = writeln!(out, "wrote {png} ({}x{})", run.image.width, run.image.height);
    Ok(0)
}

fn fmt(args: &[String], out: &mut dyn Write, err: &mut dyn Write) -> Result<i32, String> {
    let (flags, files): (Vec<_>, Vec<_>) = args.iter().partition(|a| a.starts_with("--"));
    let dry = match flags.as_slice() {
        [] => false,
        [f] if f.as_str() == "--check" => true,
        _ => return Err("fmt takes only --check".into()),
    };
    if files.is_empty() {
        return Err("fmt needs a file".into());
    }
    let mut dirty = 0;
    for file in files {
        let src = std::fs::read_to_string(file).map_err(|e| format!("{file}: {e}"))?;
        let formatted = format(&src);
        if formatted == src {
            continue;
        }
        dirty += 1;
        if dry {
            let _ = writeln!(err, "would reformat {file}");
        } else {
            std::fs::write(file, formatted).map_err(|e| format!("{file}: {e}"))?;
            let _ = writeln!(out, "formatted {file}");
        }
    }
    Ok(i32::from(dry && dirty > 0))
}

fn docs(args: &[String], out: &mut dyn Write) -> Result<i32, String> {
    match args {
        [] => {
            for (name, _) in GUIDES {
                let _ = writeln!(out, "{name}");
            }
            Ok(0)
        }
        [topic] => {
            let (_, text) = GUIDES.iter().find(|(n, _)| n == topic).ok_or_else(|| {
                let names: Vec<_> = GUIDES.iter().map(|(n, _)| *n).collect();
                format!("no guide `{topic}`, the guides are: {}", names.join(", "))
            })?;
            let _ = write!(out, "{text}");
            Ok(0)
        }
        _ => Err("docs takes at most one topic".into()),
    }
}

fn new(args: &[String], out: &mut dyn Write, _: &mut dyn Write) -> Result<i32, String> {
    let [file] = args else { return Err("new takes one file".into()) };
    if Path::new(file).exists() {
        return Err(format!("{file} already exists"));
    }
    std::fs::write(file, TEMPLATE).map_err(|e| format!("{file}: {e}"))?;
    let _ = writeln!(out, "wrote {file}");
    Ok(0)
}

/// Re-indents ACN source by four spaces per open brace, parenthesis or bracket, trims trailing space and keeps at most one blank line in a row. Strings and comments are never touched inside; the result is the same when formatted again.
pub fn format(src: &str) -> String {
    let (mut depth, mut block, mut blank) = (0usize, false, true);
    let mut out = String::new();
    for line in src.lines() {
        let trimmed = line.trim();
        if block {
            out.push_str(line.trim_end());
            out.push('\n');
            block = !trimmed.contains("*/");
            blank = false;
            continue;
        }
        if trimmed.is_empty() {
            if !blank {
                out.push('\n');
            }
            blank = true;
            continue;
        }
        blank = false;
        let (mut opens, mut closes, mut leading) = (0usize, 0usize, true);
        let (mut in_string, mut chars) = (false, trimmed.chars().peekable());
        while let Some(c) = chars.next() {
            match (in_string, c) {
                (true, '\\') => {
                    chars.next();
                }
                (true, '"') => in_string = false,
                (true, _) => {}
                (false, '"') => in_string = true,
                (false, '/') if chars.peek() == Some(&'/') => break,
                (false, '/') if chars.peek() == Some(&'*') => {
                    chars.next();
                    let mut rest = String::new();
                    rest.extend(chars.by_ref());
                    block = !rest.contains("*/");
                    break;
                }
                (false, '{' | '(' | '[') => opens += 1,
                (false, '}' | ')' | ']') => {
                    if leading && closes == 0 && opens == 0 {
                        closes += 1;
                    } else if opens > 0 {
                        opens -= 1;
                    } else {
                        closes += 1;
                    }
                }
                _ => {}
            }
            if !c.is_whitespace() && !matches!(c, '}' | ')' | ']') {
                leading = false;
            }
        }
        let starts_closed = trimmed.starts_with(['}', ')', ']']);
        let indent = if starts_closed { depth.saturating_sub(1) } else { depth };
        out.push_str(&" ".repeat(indent * 4));
        out.push_str(trimmed);
        out.push('\n');
        depth = (depth + opens).saturating_sub(closes);
    }
    while out.ends_with("\n\n") {
        out.pop();
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn exec(args: &[&str]) -> (i32, String, String) {
        let args: Vec<String> = args.iter().map(|s| s.to_string()).collect();
        let (mut out, mut err) = (Vec::new(), Vec::new());
        let code = run(&args, &mut out, &mut err);
        (code, String::from_utf8(out).unwrap(), String::from_utf8(err).unwrap())
    }

    #[test]
    fn format_indents_trims_and_is_idempotent() {
        let sl = "/".repeat(2);
        let messy = format!(
            "acn 0\nSlider {{  \nid: a\n\n\n   width: f(\n1,\n2\n   )\n{sl} }}\n  Behavior on x {{ Tween {{ duration: 5 }} }}\ntext: \"}}{{\"\n  }}\n"
        );
        let want = format!(
            "acn 0\nSlider {{\n    id: a\n\n    width: f(\n        1,\n        2\n    )\n    {sl} }}\n    Behavior on x {{ Tween {{ duration: 5 }} }}\n    text: \"}}{{\"\n}}\n"
        );
        assert_eq!(super::format(&messy), want);
        assert_eq!(super::format(&want), want);
    }

    #[test]
    fn new_then_check_then_fmt_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("a.acn");
        let file = file.to_str().unwrap();
        assert_eq!(exec(&["new", file]).0, 0);
        assert_eq!(exec(&["new", file]).0, 2);
        assert_eq!(exec(&["check", file]).0, 0);
        assert_eq!(exec(&["fmt", "--check", file]).0, 0);
        std::fs::write(file, "Slider {\nwidht: 1\n}\n").unwrap();
        let (code, _, err) = exec(&["check", file]);
        assert_eq!(code, 1);
        assert!(err.contains("unknown property `widht`") && err.contains("did you mean `width`?"), "{err}");
        assert_eq!(exec(&["fmt", "--check", file]).0, 1);
        assert_eq!(exec(&["fmt", file]).0, 0);
        assert_eq!(exec(&["fmt", "--check", file]).0, 0);
    }

    #[test]
    fn globals_are_declared_on_the_command_line() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("a.acn");
        std::fs::write(&file, "Slider { value: volume }\n").unwrap();
        let file = file.to_str().unwrap();
        assert_eq!(exec(&["check", file]).0, 1);
        assert_eq!(exec(&["check", "--global", "volume", file]).0, 0);
    }

    #[test]
    fn shot_renders_a_file_and_reports_errors() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("a.acn");
        let png = dir.path().join("a.png");
        let (file, png) = (file.to_str().unwrap(), png.to_str().unwrap());
        std::fs::write(file, "Slider { width: 200\nvalue: 0.5 }\n").unwrap();
        let Ok(_) = Headless::new(8, 8, 1.0) else { return };
        let (code, out, err) = exec(&["shot", "--size", "240x60", "--time", "100", file, png]);
        assert_eq!((code, err.as_str()), (0, ""), "{out}");
        let img = image::open(png).unwrap();
        assert_eq!((img.width(), img.height()), (240, 60));
        std::fs::write(file, "Slider { widht: 1 }\n").unwrap();
        std::fs::write(file, "Slider {\nwidth: 200\nBehavior on opacity { Tween { duration: 500 } }\n}\n").unwrap();
        let (code, _, err) = exec(&["shot", "--reduced-motion", file, png]);
        assert_eq!((code, err.as_str()), (0, ""));
        acanthus_anim::set_reduced_motion(false);
        std::fs::write(file, "Slider { widht: 1 }\n").unwrap();
        assert_eq!(exec(&["shot", file, png]).0, 1);
        assert_eq!(exec(&["shot", file]).0, 2);
    }

    #[test]
    fn docs_lists_and_prints_guides() {
        let (code, out, _) = exec(&["docs"]);
        assert_eq!((code, out.as_str()), (0, "acn-spec\nversioning\n"));
        let (code, out, _) = exec(&["docs", "acn-spec"]);
        assert_eq!(code, 0);
        assert!(out.starts_with("# "));
        let (code, _, err) = exec(&["docs", "acn-spc"]);
        assert_eq!(code, 2);
        assert!(err.contains("acn-spec"));
    }

    #[test]
    fn unknown_commands_print_usage() {
        let (code, _, err) = exec(&["frobnicate"]);
        assert_eq!(code, 2);
        assert!(err.contains("usage:"));
    }
}
