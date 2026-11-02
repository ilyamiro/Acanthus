use acanthus_widgets::standard_registry;
use std::io::Write;

fn enclosing(text: &str) -> Option<String> {
    let mut stack: Vec<Option<String>> = Vec::new();
    let mut word = String::new();
    let mut last = None;
    let mut in_string = false;
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if in_string {
            if c == '\\' {
                chars.next();
            } else if c == '"' {
                in_string = false;
            }
            continue;
        }
        if c.is_alphanumeric() || c == '_' {
            word.push(c);
            continue;
        }
        if !word.is_empty() {
            last = Some(std::mem::take(&mut word));
        }
        match c {
            '"' => in_string = true,
            '/' if chars.peek() == Some(&'/') => {
                for d in chars.by_ref() {
                    if d == '\n' {
                        break;
                    }
                }
            }
            '{' => stack.push(last.take()),
            '}' => {
                stack.pop();
                last = None;
            }
            c if c.is_whitespace() => {}
            _ => last = None,
        }
    }
    stack.into_iter().rev().flatten().next()
}

fn prefix(text: &str) -> &str {
    let start = text.rfind(|c: char| !(c.is_alphanumeric() || c == '_')).map_or(0, |i| i + 1);
    &text[start..]
}

pub(crate) fn run(args: &[String], out: &mut dyn Write) -> Result<i32, String> {
    let [file, line, col] = args else { return Err("complete takes FILE LINE COLUMN".into()) };
    let src = std::fs::read_to_string(file).map_err(|e| format!("{file}: {e}"))?;
    let (line, col): (usize, usize) = (
        line.parse().map_err(|_| "LINE is a number from 1".to_owned())?,
        col.parse().map_err(|_| "COLUMN is a number from 1".to_owned())?,
    );
    let mut before = String::new();
    for (i, text) in src.split_inclusive('\n').enumerate() {
        if i + 1 < line {
            before.push_str(text);
        } else {
            before.extend(text.chars().take(col.saturating_sub(1)));
            break;
        }
    }
    let typed = prefix(&before).to_owned();
    let registry = standard_registry();
    let parent = enclosing(&before[..before.len() - typed.len()]);
    let mut rows: Vec<(&str, String, String)> = Vec::new();
    if let Some(component) = parent.as_deref().and_then(|name| registry.get(name)) {
        for p in component.props() {
            rows.push(("prop", p.name.to_owned(), p.kind.word().to_owned()));
        }
    }
    if parent.as_deref().is_none_or(|name| registry.get(name).is_some() || name == "Repeater") {
        for name in registry.names() {
            rows.push(("component", name.to_owned(), String::new()));
        }
    }
    rows.retain(|(_, name, _)| name.starts_with(&typed));
    rows.sort();
    for (kind, name, detail) in rows {
        let _ = writeln!(out, "{kind}\t{name}\t{detail}");
    }
    Ok(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn complete(src: &str, line: &str, col: &str) -> String {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("a.acn");
        std::fs::write(&file, src).unwrap();
        let mut out = Vec::new();
        run(&[file.display().to_string(), line.into(), col.into()], &mut out).unwrap();
        String::from_utf8(out).unwrap()
    }

    #[test]
    fn components_at_the_top_and_props_inside_a_component() {
        let top = complete("acn 0\nSl", "2", "3");
        assert!(top.contains("component\tSlider") && !top.contains("prop\t"));
        let inside = complete("acn 0\nSlider {\n  wi\n}\n", "3", "5");
        assert!(inside.contains("prop\twidth\tnumber"), "{inside}");
        assert!(!inside.contains("component\t"), "{inside}");
    }

    #[test]
    fn braces_in_strings_and_comments_do_not_count() {
        let text = complete(&format!("Slider {{\n  value: \"}}\"\n  {}\n  ", "/".repeat(2) + " }"), "4", "3");
        assert!(text.contains("prop\tvalue"), "{text}");
    }
}
