use serde_json::{Map, Value};

pub fn get_path<'a>(root: &'a Value, path: &str) -> Option<&'a Value> {
    if path.is_empty() {
        return Some(root);
    }
    path.split('.').try_fold(root, |cur, part| cur.as_object()?.get(part))
}

pub fn set_path(root: &mut Value, path: &str, value: Value) {
    let mut parts = path.split('.').peekable();
    let mut cur = root;
    while let Some(part) = parts.next() {
        if !cur.is_object() {
            *cur = Value::Object(Map::new());
        }
        let map = cur.as_object_mut().expect("object");
        if parts.peek().is_none() {
            map.insert(part.to_owned(), value);
            return;
        }
        cur = map.entry(part.to_owned()).or_insert_with(|| Value::Object(Map::new()));
    }
}

pub fn remove_path(root: &mut Value, path: &str) -> bool {
    let Some((parent, leaf)) = path.rsplit_once('.').map(|(p, l)| (Some(p), l)).or(Some((None, path))) else {
        return false;
    };
    let target = match parent {
        Some(p) => match get_path_mut(root, p) {
            Some(v) => v,
            None => return false,
        },
        None => root,
    };
    target.as_object_mut().is_some_and(|m| m.shift_remove(leaf).is_some())
}

fn get_path_mut<'a>(root: &'a mut Value, path: &str) -> Option<&'a mut Value> {
    path.split('.').try_fold(root, |cur, part| cur.as_object_mut()?.get_mut(part))
}

pub fn parse_scalar(text: &str) -> Value {
    serde_json::from_str(text).unwrap_or_else(|_| Value::String(text.to_owned()))
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn gets_nested_and_missing_paths() {
        let v = json!({"bar": {"height": 40, "modules": {"left": ["a"]}}});
        assert_eq!(get_path(&v, "bar.height"), Some(&json!(40)));
        assert_eq!(get_path(&v, "bar.modules.left"), Some(&json!(["a"])));
        assert_eq!(get_path(&v, "bar.nope.x"), None);
        assert_eq!(get_path(&v, "bar.height.x"), None);
        assert_eq!(get_path(&v, ""), Some(&v));
    }

    #[test]
    fn set_creates_intermediate_objects_and_replaces_scalars() {
        let mut v = json!({"a": 1});
        set_path(&mut v, "a.b.c", json!(true));
        assert_eq!(v, json!({"a": {"b": {"c": true}}}));
        set_path(&mut v, "x", json!("y"));
        assert_eq!(v["x"], "y");
    }

    #[test]
    fn removes_leaves() {
        let mut v = json!({"a": {"b": 1, "c": 2}, "d": 3});
        assert!(remove_path(&mut v, "a.b"));
        assert!(remove_path(&mut v, "d"));
        assert!(!remove_path(&mut v, "a.zzz"));
        assert_eq!(v, json!({"a": {"c": 2}}));
    }

    #[test]
    fn scalars_parse_as_json_or_fall_back_to_strings() {
        assert_eq!(parse_scalar("42"), json!(42));
        assert_eq!(parse_scalar("true"), json!(true));
        assert_eq!(parse_scalar("hello"), json!("hello"));
        assert_eq!(parse_scalar("[1,2]"), json!([1, 2]));
    }
}
