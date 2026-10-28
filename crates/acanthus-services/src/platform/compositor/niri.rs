use std::collections::BTreeMap;

use serde_json::Value;

use super::types::{Snapshot, Window, Workspace};

#[derive(Debug, Default)]
pub struct NiriModel {
    workspaces: BTreeMap<i64, Value>,
    windows: BTreeMap<i64, Value>,
    keyboard_layouts: Vec<String>,
    keyboard_layout_index: usize,
}

fn by_id(list: Option<&Value>) -> BTreeMap<i64, Value> {
    list.and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|item| Some((item.get("id")?.as_i64()?, item.clone())))
        .collect()
}

fn set_bool(value: &mut Value, key: &str, flag: bool) {
    if let Some(object) = value.as_object_mut() {
        object.insert(key.to_owned(), Value::Bool(flag));
    }
}

impl NiriModel {
    pub fn apply(&mut self, event: &Value) -> bool {
        let Some(object) = event.as_object() else { return false };
        if let Some(p) = object.get("WorkspacesChanged") {
            self.workspaces = by_id(p.get("workspaces"));
        } else if let Some(p) = object.get("WorkspaceActivated") {
            let (Some(id), focused) = (p.get("id").and_then(Value::as_i64), p.get("focused").and_then(Value::as_bool))
            else {
                return false;
            };
            let Some(target_output) = self.workspaces.get(&id).map(|w| w.get("output").cloned()) else {
                return false;
            };
            for (wid, workspace) in &mut self.workspaces {
                if workspace.get("output") == target_output.as_ref() {
                    set_bool(workspace, "is_active", *wid == id);
                    if focused != Some(false) {
                        set_bool(workspace, "is_focused", *wid == id);
                    }
                } else if focused != Some(false) {
                    set_bool(workspace, "is_focused", false);
                }
            }
        } else if let Some(p) = object.get("WorkspaceActiveWindowChanged") {
            let Some(id) = p.get("workspace_id").and_then(Value::as_i64) else { return false };
            let Some(workspace) = self.workspaces.get_mut(&id).and_then(Value::as_object_mut) else { return false };
            workspace.insert("active_window_id".into(), p.get("active_window_id").cloned().unwrap_or(Value::Null));
        } else if let Some(p) = object.get("WindowsChanged") {
            self.windows = by_id(p.get("windows"));
        } else if let Some(p) = object.get("WindowOpenedOrChanged") {
            let Some(window) = p.get("window") else { return false };
            let Some(id) = window.get("id").and_then(Value::as_i64) else { return false };
            self.windows.insert(id, window.clone());
        } else if let Some(p) = object.get("WindowClosed") {
            let Some(id) = p.get("id").and_then(Value::as_i64) else { return false };
            self.windows.remove(&id);
        } else if let Some(p) = object.get("WindowFocusChanged") {
            let focused = p.get("id").and_then(Value::as_i64).unwrap_or(-1);
            for (id, window) in &mut self.windows {
                set_bool(window, "is_focused", *id == focused);
            }
        } else if let Some(p) = object.get("KeyboardLayoutsChanged") {
            let layouts = p.get("keyboard_layouts");
            self.keyboard_layouts = layouts
                .and_then(|l| l.get("names"))
                .and_then(Value::as_array)
                .map(|names| names.iter().filter_map(|n| n.as_str().map(str::to_owned)).collect())
                .unwrap_or_default();
            self.keyboard_layout_index =
                layouts.and_then(|l| l.get("current_idx")).and_then(Value::as_u64).unwrap_or(0) as usize;
        } else if let Some(p) = object.get("KeyboardLayoutSwitched") {
            self.keyboard_layout_index = p.get("idx").and_then(Value::as_u64).unwrap_or(0) as usize;
        } else {
            return false;
        }
        true
    }

    pub fn snapshot(&self) -> Snapshot {
        let workspace_has_window: Vec<i64> =
            self.windows.values().filter_map(|w| w.get("workspace_id").and_then(Value::as_i64)).collect();

        let mut workspaces = Vec::new();
        let mut focused_output = String::new();
        for (id, value) in &self.workspaces {
            let index = value
                .get("idx")
                .and_then(Value::as_i64)
                .or_else(|| value.get("id").and_then(Value::as_i64))
                .unwrap_or(1) as i32
                - 1;
            if index < 0 {
                continue;
            }
            let flag = |key: &str| value.get(key).and_then(Value::as_bool).unwrap_or(false);
            let has_active_window = value.get("active_window_id").is_some_and(|v| !v.is_null());
            let output = value.get("output").and_then(Value::as_str).unwrap_or("").to_owned();
            if flag("is_focused") {
                focused_output = output.clone();
            }
            workspaces.push(Workspace {
                id: *id,
                index,
                name: value.get("name").and_then(Value::as_str).unwrap_or("").to_owned(),
                output,
                active: flag("is_active"),
                focused: flag("is_focused"),
                occupied: has_active_window || workspace_has_window.contains(id),
                urgent: flag("is_urgent"),
                has_fullscreen: false,
            });
        }

        let windows = self
            .windows
            .iter()
            .map(|(id, value)| {
                let workspace_id = value.get("workspace_id").and_then(Value::as_i64);
                let workspace = workspace_id.and_then(|wid| workspaces.iter().find(|w| w.id == wid));
                let flag = |key: &str| value.get(key).and_then(Value::as_bool).unwrap_or(false);
                Window {
                    id: format!("niri:{id}"),
                    native_id: id.to_string(),
                    app_id: value.get("app_id").and_then(Value::as_str).unwrap_or("").to_owned(),
                    title: value.get("title").and_then(Value::as_str).unwrap_or("").to_owned(),
                    workspace: workspace.map_or(-1, |w| w.index),
                    workspace_name: workspace.map(|w| w.name.clone()).unwrap_or_default(),
                    output: workspace.map(|w| w.output.clone()).unwrap_or_default(),
                    focused: flag("is_focused"),
                    fullscreen: false,
                    floating: flag("is_floating"),
                    urgent: flag("is_urgent"),
                    pid: value.get("pid").and_then(Value::as_i64),
                }
            })
            .collect();

        Snapshot {
            workspaces,
            windows,
            focused_output,
            keyboard_layouts: self.keyboard_layouts.clone(),
            keyboard_layout_index: self.keyboard_layout_index,
        }
    }
}

pub fn focus_workspace_request(index: u32) -> Value {
    serde_json::json!({"Action": {"FocusWorkspace": {"reference": {"Index": index}}}})
}

pub fn focus_window_request(id: i64) -> Value {
    serde_json::json!({"Action": {"FocusWindow": {"id": id}}})
}

pub fn close_window_request(id: i64) -> Value {
    serde_json::json!({"Action": {"CloseWindow": {"id": id}}})
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn seeded() -> NiriModel {
        let mut model = NiriModel::default();
        model.apply(&json!({"WorkspacesChanged": {"workspaces": [
            {"id": 11, "idx": 1, "name": null, "output": "eDP-1", "is_active": true, "is_focused": true, "active_window_id": 100},
            {"id": 12, "idx": 2, "name": "web", "output": "eDP-1", "is_active": false, "is_focused": false, "active_window_id": null},
            {"id": 21, "idx": 1, "name": null, "output": "HDMI-A-1", "is_active": true, "is_focused": false, "active_window_id": null}
        ]}}));
        model.apply(&json!({"WindowsChanged": {"windows": [
            {"id": 100, "title": "term", "app_id": "kitty", "pid": 7, "workspace_id": 11, "is_focused": true, "is_floating": false},
            {"id": 101, "title": "docs", "app_id": "firefox", "pid": 8, "workspace_id": 12, "is_focused": false, "is_floating": true}
        ]}}));
        model
    }

    #[test]
    fn snapshot_maps_workspaces_windows_and_occupancy() {
        let snapshot = seeded().snapshot();
        assert_eq!(snapshot.workspaces.len(), 3);
        let web = snapshot.workspaces.iter().find(|w| w.name == "web").unwrap();
        assert_eq!((web.index, web.occupied, web.active), (1, true, false));
        let empty = snapshot.workspaces.iter().find(|w| w.output == "HDMI-A-1").unwrap();
        assert!(!empty.occupied);
        assert_eq!(snapshot.focused_output, "eDP-1");
        assert_eq!(snapshot.active_index(), 0);
        assert_eq!(snapshot.active_index_for("HDMI-A-1"), 0);
        let docs = snapshot.windows.iter().find(|w| w.app_id == "firefox").unwrap();
        assert_eq!((docs.workspace, docs.floating, docs.workspace_name.as_str()), (1, true, "web"));
        assert_eq!(docs.id, "niri:101");
        assert_eq!(snapshot.focused_window().unwrap().title, "term");
    }

    #[test]
    fn workspace_activation_moves_focus_within_the_output_only() {
        let mut model = seeded();
        assert!(model.apply(&json!({"WorkspaceActivated": {"id": 12, "focused": true}})));
        let snapshot = model.snapshot();
        let by_id = |id: i64| snapshot.workspaces.iter().find(|w| w.id == id).unwrap().clone();
        assert!(by_id(12).active && by_id(12).focused);
        assert!(!by_id(11).active && !by_id(11).focused);
        assert!(by_id(21).active && !by_id(21).focused);
        assert_eq!(snapshot.active_index(), 1);

        model.apply(&json!({"WorkspaceActivated": {"id": 21, "focused": false}}));
        let snapshot = model.snapshot();
        assert!(snapshot.workspaces.iter().find(|w| w.id == 12).unwrap().focused);
    }

    #[test]
    fn window_lifecycle_events() {
        let mut model = seeded();
        model.apply(&json!({"WindowOpenedOrChanged": {"window": {"id": 102, "title": "new", "app_id": "foot", "workspace_id": 21, "is_focused": false}}}));
        assert!(model.snapshot().workspaces.iter().find(|w| w.id == 21).unwrap().occupied);
        model.apply(&json!({"WindowFocusChanged": {"id": 102}}));
        assert_eq!(model.snapshot().focused_window().unwrap().title, "new");
        model.apply(&json!({"WindowFocusChanged": {"id": null}}));
        assert!(model.snapshot().focused_window().is_none());
        model.apply(&json!({"WindowClosed": {"id": 102}}));
        assert!(!model.snapshot().workspaces.iter().find(|w| w.id == 21).unwrap().occupied);
        assert_eq!(model.snapshot().windows.len(), 2);
    }

    #[test]
    fn active_window_change_toggles_occupancy_without_window_entries() {
        let mut model = NiriModel::default();
        model.apply(&json!({"WorkspacesChanged": {"workspaces": [{"id": 1, "idx": 1, "output": "A", "is_active": true, "is_focused": true, "active_window_id": null}]}}));
        assert!(!model.snapshot().workspaces[0].occupied);
        model.apply(&json!({"WorkspaceActiveWindowChanged": {"workspace_id": 1, "active_window_id": 5}}));
        assert!(model.snapshot().workspaces[0].occupied);
    }

    #[test]
    fn keyboard_layout_events() {
        let mut model = NiriModel::default();
        model.apply(&json!({"KeyboardLayoutsChanged": {"keyboard_layouts": {"names": ["English (US)", "Ukrainian"], "current_idx": 0}}}));
        assert_eq!(model.snapshot().short_keyboard_layout(), "EN");
        model.apply(&json!({"KeyboardLayoutSwitched": {"idx": 1}}));
        assert_eq!(model.snapshot().keyboard_layout(), "Ukrainian");
    }

    #[test]
    fn unknown_and_malformed_events_are_ignored() {
        let mut model = seeded();
        let before = model.snapshot();
        assert!(!model.apply(&json!({"OverviewOpenedOrClosed": {"is_open": true}})));
        assert!(!model.apply(&json!("not an object")));
        assert!(!model.apply(&json!({"WorkspaceActivated": {"id": 999}})));
        assert_eq!(model.snapshot(), before);
    }

    #[test]
    fn request_shapes_match_the_niri_ipc_schema() {
        assert_eq!(focus_workspace_request(3), json!({"Action": {"FocusWorkspace": {"reference": {"Index": 3}}}}));
        assert_eq!(focus_window_request(9), json!({"Action": {"FocusWindow": {"id": 9}}}));
        assert_eq!(close_window_request(9), json!({"Action": {"CloseWindow": {"id": 9}}}));
    }
}
