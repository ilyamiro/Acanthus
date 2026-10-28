use serde_json::Value;

use super::types::{Snapshot, Window, Workspace};

#[derive(Debug, Default)]
pub struct MangoModel {
    tags: Value,
    clients: Value,
}

impl MangoModel {
    pub fn set_tags(&mut self, payload: Value) {
        self.tags = payload;
    }

    pub fn set_clients(&mut self, payload: Value) {
        self.clients = payload;
    }

    pub fn snapshot(&self) -> Snapshot {
        let mut workspaces = Vec::new();
        let mut focused_output = String::new();
        let mut first_active: Option<String> = None;
        for monitor in self.tags.get("all_tags").and_then(Value::as_array).into_iter().flatten() {
            let Some(tags) = monitor.get("tags").and_then(Value::as_array) else { continue };
            let output = monitor.get("monitor").and_then(Value::as_str).unwrap_or("").to_owned();
            let mut active_here = None;
            for (position, tag) in tags.iter().enumerate() {
                let index = tag.get("index").and_then(Value::as_i64).unwrap_or(position as i64 + 1) as i32 - 1;
                let is_active = tag.get("is_active").and_then(Value::as_bool).unwrap_or(false) && active_here.is_none();
                if is_active {
                    active_here = Some(index);
                }
                workspaces.push(Workspace {
                    id: i64::from(index) + 1,
                    index,
                    name: (index + 1).to_string(),
                    output: output.clone(),
                    active: is_active,
                    focused: false,
                    occupied: tag.get("client_count").and_then(Value::as_i64).unwrap_or(0) > 0,
                    urgent: false,
                    has_fullscreen: false,
                });
            }
            if active_here.is_some() && first_active.is_none() {
                first_active = Some(output);
            }
        }
        if let Some(output) = first_active {
            focused_output = output.clone();
            if let Some(w) = workspaces.iter_mut().find(|w| w.output == output && w.active) {
                w.focused = true;
            }
        }

        let windows = self
            .clients
            .get("clients")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|c| {
                let id = c
                    .get("id")
                    .filter(|v| !v.is_null())
                    .map(|v| v.as_str().map_or_else(|| v.to_string(), str::to_owned))?;
                let tag = c
                    .get("tags")
                    .and_then(Value::as_array)
                    .and_then(|t| t.first())
                    .and_then(Value::as_i64)
                    .map_or(-1, |t| (t - 1) as i32);
                Some(Window {
                    id: format!("mango:{id}"),
                    native_id: id,
                    app_id: c.get("appid").and_then(Value::as_str).unwrap_or("").to_owned(),
                    title: c.get("title").and_then(Value::as_str).unwrap_or("").to_owned(),
                    workspace: tag,
                    workspace_name: if tag >= 0 { (tag + 1).to_string() } else { String::new() },
                    output: c.get("monitor").and_then(Value::as_str).unwrap_or("").to_owned(),
                    focused: c.get("is_focused").and_then(Value::as_bool).unwrap_or(false),
                    fullscreen: c.get("is_fullscreen").and_then(Value::as_bool).unwrap_or(false),
                    floating: c.get("is_floating").and_then(Value::as_bool).unwrap_or(false),
                    urgent: false,
                    pid: c.get("pid").and_then(Value::as_i64),
                })
            })
            .collect();

        Snapshot { workspaces, windows, focused_output, keyboard_layouts: Vec::new(), keyboard_layout_index: 0 }
    }
}

pub fn view_command(index: u32) -> Vec<String> {
    vec!["mmsg".into(), "dispatch".into(), format!("view,{}", index + 1)]
}

pub fn focus_command(id: &str) -> Vec<String> {
    vec!["mmsg".into(), "dispatch".into(), "focusid".into(), format!("client,{id}")]
}

pub fn close_command(id: &str) -> Vec<String> {
    vec!["mmsg".into(), "dispatch".into(), "killclient".into(), format!("client,{id}")]
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn tags_and_clients_become_a_snapshot() {
        let mut model = MangoModel::default();
        model.set_tags(json!({"all_tags": [
            {"monitor": "eDP-1", "tags": [
                {"index": 1, "client_count": 2, "is_active": false},
                {"index": 2, "client_count": 0, "is_active": true},
                {"index": 3, "client_count": 1, "is_active": true}
            ]},
            {"monitor": "HDMI-A-1", "tags": [{"index": 1, "client_count": 0, "is_active": true}]}
        ]}));
        model.set_clients(json!({"clients": [
            {"id": 7, "appid": "foot", "title": "sh", "is_focused": true, "tags": [1], "monitor": "eDP-1"},
            {"id": "x", "appid": "mpv", "tags": [], "monitor": "HDMI-A-1"},
            {"appid": "no id"}
        ]}));
        let snapshot = model.snapshot();
        assert_eq!(snapshot.workspaces.len(), 4);
        let active: Vec<(&str, i32)> =
            snapshot.workspaces.iter().filter(|w| w.active).map(|w| (w.output.as_str(), w.index)).collect();
        assert_eq!(active, [("eDP-1", 1), ("HDMI-A-1", 0)]);
        assert_eq!(snapshot.focused_output, "eDP-1");
        assert_eq!(snapshot.active_index(), 1);
        assert!(snapshot.is_occupied("eDP-1", 0));
        assert!(!snapshot.is_occupied("eDP-1", 1));
        assert_eq!(snapshot.windows.len(), 2);
        let foot = &snapshot.windows[0];
        assert_eq!(
            (foot.id.as_str(), foot.workspace, foot.workspace_name.as_str(), foot.focused),
            ("mango:7", 0, "1", true)
        );
        assert_eq!(snapshot.windows[1].workspace, -1);
    }

    #[test]
    fn empty_payloads_give_an_empty_snapshot() {
        assert_eq!(MangoModel::default().snapshot(), Snapshot::default());
    }

    #[test]
    fn command_lines_match_the_original_dispatchers() {
        assert_eq!(view_command(2), ["mmsg", "dispatch", "view,3"]);
        assert_eq!(focus_command("7"), ["mmsg", "dispatch", "focusid", "client,7"]);
        assert_eq!(close_command("7"), ["mmsg", "dispatch", "killclient", "client,7"]);
    }
}
