use acanthus_markup::Functions;
use acanthus_widgets::standard_registry;
use std::collections::BTreeSet;
use std::io::Write;

struct Words {
    components: Vec<String>,
    props: Vec<String>,
    functions: Vec<String>,
}

fn words() -> Words {
    let registry = standard_registry();
    let mut components: BTreeSet<String> = registry.names().iter().map(|n| n.to_string()).collect();
    components.extend(["Repeater", "Tween", "Spring"].map(String::from));
    let mut props = BTreeSet::new();
    for name in registry.names() {
        if let Some(c) = registry.get(name) {
            props.extend(c.props().iter().map(|p| p.name.to_string()));
        }
    }
    let mut functions: BTreeSet<String> = Functions::standard().names().into_iter().map(String::from).collect();
    functions.extend(["t", "action"].map(String::from));
    Words {
        components: components.into_iter().collect(),
        props: props.into_iter().filter(|p| !p.starts_with("on_")).collect(),
        functions: functions.into_iter().collect(),
    }
}

pub(crate) fn run(args: &[String], out: &mut dyn Write) -> Result<i32, String> {
    let [format] = args else { return Err("grammar takes one format: vim or textmate".into()) };
    let w = words();
    let text = match format.as_str() {
        "vim" => vim(&w),
        "textmate" => textmate(&w),
        other => return Err(format!("unknown format `{other}`, use vim or textmate")),
    };
    let _ = write!(out, "{text}");
    Ok(0)
}

fn vim(w: &Words) -> String {
    let list = |v: &[String]| v.join(" ");
    format!(
        "if exists(\"b:current_syntax\")\n  finish\nendif\n\
         syntax keyword acnKeyword acn id property Behavior on true false null\n\
         syntax keyword acnComponent {}\n\
         syntax keyword acnProp {}\n\
         syntax keyword acnFunction {}\n\
         syntax match acnHandler /\\<on_\\w\\+/\n\
         syntax match acnNumber /\\<\\d\\+\\(\\.\\d\\+\\)\\=\\>/\n\
         syntax match acnColor /#\\x\\{{3,8}}\\>/\n\
         syntax region acnString start=/\"/ skip=/\\\\\"/ end=/\"/\n\
         syntax match acnComment /\\/\\/.*$/\n\
         highlight default link acnKeyword Keyword\n\
         highlight default link acnComponent Type\n\
         highlight default link acnProp Identifier\n\
         highlight default link acnFunction Function\n\
         highlight default link acnHandler Special\n\
         highlight default link acnNumber Number\n\
         highlight default link acnColor Constant\n\
         highlight default link acnString String\n\
         highlight default link acnComment Comment\n\
         let b:current_syntax = \"acn\"\n",
        list(&w.components),
        list(&w.props),
        list(&w.functions)
    )
}

fn textmate(w: &Words) -> String {
    let alt = |v: &[String]| v.join("|");
    format!(
        "{{\n  \"scopeName\": \"source.acn\",\n  \"patterns\": [\n\
         \x20   {{ \"name\": \"comment.line.double-slash.acn\", \"match\": \"/{{2}}.*$\" }},\n\
         \x20   {{ \"name\": \"string.quoted.double.acn\", \"begin\": \"\\\"\", \"end\": \"\\\"\", \"patterns\": [{{ \"match\": \"\\\\\\\\.\" }}] }},\n\
         \x20   {{ \"name\": \"constant.other.color.acn\", \"match\": \"#[0-9a-fA-F]{{3,8}}\\\\b\" }},\n\
         \x20   {{ \"name\": \"constant.numeric.acn\", \"match\": \"\\\\b\\\\d+(\\\\.\\\\d+)?\\\\b\" }},\n\
         \x20   {{ \"name\": \"keyword.control.acn\", \"match\": \"\\\\b(acn|id|property|Behavior|on|true|false|null)\\\\b\" }},\n\
         \x20   {{ \"name\": \"entity.name.type.acn\", \"match\": \"\\\\b({})\\\\b\" }},\n\
         \x20   {{ \"name\": \"support.function.acn\", \"match\": \"\\\\b({})(?=\\\\()\" }},\n\
         \x20   {{ \"name\": \"entity.name.function.handler.acn\", \"match\": \"\\\\bon_\\\\w+\" }},\n\
         \x20   {{ \"name\": \"variable.other.property.acn\", \"match\": \"\\\\b({})(?=\\\\s*:)\" }}\n\
         \x20 ]\n}}\n",
        alt(&w.components),
        alt(&w.functions),
        alt(&w.props)
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn grammars_list_the_registry_vocabulary() {
        let w = words();
        assert!(w.components.contains(&"Slider".to_string()) && w.components.contains(&"Repeater".to_string()));
        assert!(w.props.contains(&"value".to_string()) && !w.props.iter().any(|p| p.starts_with("on_")));
        for text in [vim(&w), textmate(&w)] {
            assert!(text.contains("Slider") && text.contains("fmt_time") && text.contains("Behavior"));
        }
    }

    #[test]
    fn textmate_output_keeps_regex_braces() {
        let text = textmate(&words());
        assert!(text.contains("\"/{2}.*$\"") && text.contains("{3,8}"));
    }

    #[test]
    fn unknown_formats_are_usage_errors() {
        let mut out = Vec::new();
        assert!(run(&["emacs".into()], &mut out).is_err());
        assert!(run(&[], &mut out).is_err());
    }
}
