//! Tolerant serde field parsers for hand-edited settings, used with `#[serde(deserialize_with = "...")]`: settings
//! written by people or by older versions carry strings for numbers, `0`/`1` for flags and empty strings for "unset".

use serde::{Deserialize, Deserializer};
use serde_json::Value;

/// QML (JavaScript) truthiness of a JSON value: `null`, `false`, `0`, `NaN` and `""` are false, everything else true.
pub fn is_truthy(value: &Value) -> bool {
    match value {
        Value::Null => false,
        Value::Bool(b) => *b,
        Value::Number(n) => n.as_f64().is_some_and(|f| f != 0.0 && !f.is_nan()),
        Value::String(s) => !s.is_empty(),
        Value::Array(_) | Value::Object(_) => true,
    }
}

/// A switch read as text: booleans as they are, numbers true when non-zero, strings true for `"true"`, `"yes"`,
/// `"on"` and `"1"` (any case), `null` false.
pub fn is_on(value: &Value) -> bool {
    match value {
        Value::String(s) => matches!(s.trim().to_lowercase().as_str(), "true" | "yes" | "on" | "1"),
        other => is_truthy(other),
    }
}

/// A number from a JSON number or a numeric string; `None` for anything else.
pub fn number_of(value: &Value) -> Option<f64> {
    match value {
        Value::Number(n) => n.as_f64(),
        Value::String(s) => s.trim().parse::<f64>().ok(),
        _ => None,
    }
    .filter(|f| f.is_finite())
}

/// Field parser: a flag with QML truthiness ([`is_truthy`]); never fails.
pub fn truthy<'de, D: Deserializer<'de>>(deserializer: D) -> Result<bool, D::Error> {
    Value::deserialize(deserializer).map(|value| is_truthy(&value))
}

/// Field parser: a flag written as a boolean, a number or text ([`is_on`]); never fails.
pub fn switch<'de, D: Deserializer<'de>>(deserializer: D) -> Result<bool, D::Error> {
    Value::deserialize(deserializer).map(|value| is_on(&value))
}

/// Field parser: an optional number accepting numeric strings ([`number_of`]); anything else is `None`.
pub fn number<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Option<f64>, D::Error> {
    Value::deserialize(deserializer).map(|value| number_of(&value))
}

/// Field parser: an optional whole number accepting numeric strings, truncated toward zero; anything else is `None`.
pub fn integer<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Option<i64>, D::Error> {
    Value::deserialize(deserializer).map(|value| number_of(&value).map(|f| f.trunc() as i64))
}

/// Field parser: an optional text where an empty or blank string, `null` or a non-string means unset.
pub fn text<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Option<String>, D::Error> {
    Value::deserialize(deserializer).map(|value| value.as_str().filter(|s| !s.trim().is_empty()).map(str::to_owned))
}

#[cfg(test)]
mod tests {
    use serde::Deserialize;
    use serde_json::json;

    use super::*;

    #[derive(Debug, Default, PartialEq, Deserialize)]
    #[serde(default)]
    struct Loose {
        #[serde(deserialize_with = "truthy")]
        hide: bool,
        #[serde(deserialize_with = "switch")]
        pinned: bool,
        #[serde(deserialize_with = "integer")]
        count: Option<i64>,
        #[serde(deserialize_with = "number")]
        ratio: Option<f64>,
        #[serde(deserialize_with = "text")]
        style: Option<String>,
    }

    fn parse(value: Value) -> Loose {
        serde_json::from_value(value).unwrap()
    }

    #[test]
    fn truthiness_follows_qml() {
        for (value, expected) in [
            (json!(null), false),
            (json!(0), false),
            (json!(0.0), false),
            (json!(""), false),
            (json!(false), false),
            (json!(2), true),
            (json!("false"), true),
            (json!([]), true),
            (json!({}), true),
        ] {
            assert_eq!(is_truthy(&value), expected, "{value}");
        }
        assert!(parse(json!({"hide": "yes"})).hide);
        assert!(!parse(json!({"hide": null})).hide);
    }

    #[test]
    fn a_switch_reads_text() {
        for (value, expected) in [
            (json!("TRUE"), true),
            (json!(" on "), true),
            (json!("1"), true),
            (json!("false"), false),
            (json!("0"), false),
        ] {
            assert_eq!(parse(json!({ "pinned": value })).pinned, expected, "{value}");
        }
    }

    #[test]
    fn numbers_accept_strings_and_ignore_garbage() {
        let loose = parse(json!({"count": "7.9", "ratio": " 0.5 "}));
        assert_eq!((loose.count, loose.ratio), (Some(7), Some(0.5)));
        let loose = parse(json!({"count": -2.7, "ratio": "wide"}));
        assert_eq!((loose.count, loose.ratio), (Some(-2), None));
        assert_eq!(parse(json!({"count": [1]})).count, None);
    }

    #[test]
    fn blank_text_is_unset_and_missing_fields_default() {
        assert_eq!(parse(json!({"style": "  "})).style, None);
        assert_eq!(parse(json!({"style": 3})).style, None);
        assert_eq!(parse(json!({"style": "pills"})).style.as_deref(), Some("pills"));
        assert_eq!(parse(json!({})), Loose::default());
    }
}
