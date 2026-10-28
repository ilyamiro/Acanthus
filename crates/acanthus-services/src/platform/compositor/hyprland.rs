use std::io::{Read, Write};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde_json::Value;

use super::lines::LineReader;
use super::types::{Snapshot, Window, Workspace};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HyprlandPaths {
    pub requests: PathBuf,
    pub events: PathBuf,
}

impl HyprlandPaths {
    pub fn from_env(env: impl Fn(&str) -> Option<String>) -> Option<HyprlandPaths> {
        let signature = env("HYPRLAND_INSTANCE_SIGNATURE").filter(|s| !s.is_empty())?;
        let runtime = env("XDG_RUNTIME_DIR").filter(|s| !s.is_empty()).map(PathBuf::from);
        let candidates = runtime
            .map(|r| r.join("hypr").join(&signature))
            .into_iter()
            .chain(std::iter::once(PathBuf::from("/tmp/hypr").join(&signature)));
        let dir = candidates.clone().find(|d| d.join(".socket.sock").exists()).or_else(|| candidates.last())?;
        Some(HyprlandPaths { requests: dir.join(".socket.sock"), events: dir.join(".socket2.sock") })
    }
}

pub fn request(socket: &Path, command: &str) -> std::io::Result<String> {
    let mut stream = UnixStream::connect(socket)?;
    stream.set_read_timeout(Some(Duration::from_secs(2)))?;
    stream.set_write_timeout(Some(Duration::from_secs(2)))?;
    stream.write_all(command.as_bytes())?;
    let mut reply = String::new();
    stream.read_to_string(&mut reply)?;
    Ok(reply)
}

fn json_request(socket: &Path, command: &str) -> std::io::Result<Value> {
    let reply = request(socket, command)?;
    serde_json::from_str(&reply).map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))
}

pub fn fetch(socket: &Path) -> std::io::Result<Snapshot> {
    let workspaces = json_request(socket, "j/workspaces")?;
    let clients = json_request(socket, "j/clients")?;
    let monitors = json_request(socket, "j/monitors")?;
    let active = json_request(socket, "j/activewindow").unwrap_or(Value::Null);
    let devices = json_request(socket, "j/devices").unwrap_or(Value::Null);
    Ok(parse(&workspaces, &clients, &monitors, &active, &devices))
}

fn text(value: &Value, key: &str) -> String {
    value.get(key).and_then(Value::as_str).unwrap_or("").to_owned()
}

pub fn parse(workspaces: &Value, clients: &Value, monitors: &Value, active: &Value, devices: &Value) -> Snapshot {
    let monitors: Vec<&Value> = monitors.as_array().map(|m| m.iter().collect()).unwrap_or_default();
    let monitor_by_id = |id: i64| monitors.iter().find(|m| m.get("id").and_then(Value::as_i64) == Some(id));
    let focused_output = monitors
        .iter()
        .find(|m| m.get("focused").and_then(Value::as_bool) == Some(true))
        .map(|m| text(m, "name"))
        .unwrap_or_default();

    let workspace_list = workspaces
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|w| {
            let id = w.get("id")?.as_i64()?;
            if id <= 0 {
                return None;
            }
            let output = text(w, "monitor");
            let monitor = monitors.iter().find(|m| text(m, "name") == output);
            let active_here =
                monitor.and_then(|m| m.get("activeWorkspace")).and_then(|a| a.get("id")).and_then(Value::as_i64)
                    == Some(id);
            let monitor_focused = monitor.and_then(|m| m.get("focused")).and_then(Value::as_bool) == Some(true);
            Some(Workspace {
                id,
                index: (id - 1) as i32,
                name: text(w, "name"),
                output,
                active: active_here,
                focused: active_here && monitor_focused,
                occupied: w.get("windows").and_then(Value::as_i64).unwrap_or(0) > 0,
                urgent: false,
                has_fullscreen: w.get("hasfullscreen").and_then(Value::as_bool).unwrap_or(false),
            })
        })
        .collect();

    let active_address = active.get("address").and_then(Value::as_str).unwrap_or("");
    let window_list = clients
        .as_array()
        .into_iter()
        .flatten()
        .filter(|c| c.get("mapped").and_then(Value::as_bool).unwrap_or(true))
        .map(|c| {
            let address = text(c, "address");
            let workspace_id = c.get("workspace").and_then(|w| w.get("id")).and_then(Value::as_i64).unwrap_or(-1);
            let class = {
                let class = text(c, "class");
                if class.is_empty() { text(c, "initialClass") } else { class }
            };
            Window {
                id: format!("hypr:{address}"),
                focused: !active_address.is_empty() && address == active_address,
                native_id: address,
                app_id: class,
                title: text(c, "title"),
                workspace: if workspace_id > 0 { (workspace_id - 1) as i32 } else { -1 },
                workspace_name: c.get("workspace").map(|w| text(w, "name")).unwrap_or_default(),
                output: c
                    .get("monitor")
                    .and_then(Value::as_i64)
                    .and_then(monitor_by_id)
                    .map(|m| text(m, "name"))
                    .unwrap_or_default(),
                fullscreen: c.get("fullscreen").and_then(Value::as_i64).unwrap_or(0) & 2 != 0,
                floating: c.get("floating").and_then(Value::as_bool).unwrap_or(false),
                urgent: false,
                pid: c.get("pid").and_then(Value::as_i64),
            }
        })
        .collect();

    let keyboards = devices.get("keyboards").and_then(Value::as_array);
    let keymap = keyboards
        .and_then(|k| k.iter().find(|kb| kb.get("main").and_then(Value::as_bool) == Some(true)).or_else(|| k.first()))
        .map(|kb| text(kb, "active_keymap"))
        .filter(|s| !s.is_empty());

    Snapshot {
        workspaces: workspace_list,
        windows: window_list,
        focused_output,
        keyboard_layouts: keymap.into_iter().collect(),
        keyboard_layout_index: 0,
    }
}

fn accepted(reply: &str) -> bool {
    reply.trim() == "ok"
}

fn dispatch_with_fallback(socket: &Path, lua: &str, legacy: &str) -> std::io::Result<bool> {
    if accepted(&request(socket, &format!("dispatch {lua}"))?) {
        return Ok(true);
    }
    Ok(accepted(&request(socket, &format!("dispatch {legacy}"))?))
}

pub fn focus_workspace(socket: &Path, id: i64) -> std::io::Result<bool> {
    dispatch_with_fallback(socket, &format!("hl.dsp.focus({{ workspace = {id} }})"), &format!("workspace {id}"))
}

pub fn focus_window(socket: &Path, address: &str) -> std::io::Result<bool> {
    dispatch_with_fallback(
        socket,
        &format!("hl.dsp.focus({{ window = \"address:{address}\" }})"),
        &format!("focuswindow address:{address}"),
    )
}

pub fn close_window(socket: &Path, address: &str) -> std::io::Result<bool> {
    dispatch_with_fallback(
        socket,
        &format!("hl.dsp.window.close({{ window = \"address:{address}\" }})"),
        &format!("closewindow address:{address}"),
    )
}

pub fn switch_keyboard_layout(socket: &Path) -> std::io::Result<bool> {
    Ok(accepted(&request(socket, "switchxkblayout main next")?))
}

pub fn connect_events(socket: &Path, poll: Duration) -> std::io::Result<LineReader> {
    LineReader::new(UnixStream::connect(socket)?, poll)
}

#[cfg(test)]
pub mod tests {
    use std::os::unix::net::UnixListener;
    use std::sync::{Arc, Mutex};

    use serde_json::json;

    use super::*;

    pub fn sample() -> (Value, Value, Value, Value, Value) {
        (
            json!([
                {"id": 1, "name": "1", "monitor": "eDP-1", "monitorID": 0, "windows": 1, "hasfullscreen": false},
                {"id": 2, "name": "2", "monitor": "eDP-1", "monitorID": 0, "windows": 0, "hasfullscreen": false},
                {"id": 3, "name": "3", "monitor": "eDP-1", "monitorID": 0, "windows": 1, "hasfullscreen": true},
                {"id": -98, "name": "special:scratch", "monitor": "eDP-1", "windows": 1, "hasfullscreen": false},
                {"id": 4, "name": "4", "monitor": "HDMI-A-1", "monitorID": 1, "windows": 2, "hasfullscreen": false}
            ]),
            json!([
                {"address": "0x64e79f243580", "mapped": true, "workspace": {"id": 1, "name": "1"}, "floating": false, "monitor": 0, "class": "firefox", "title": "repo — Mozilla Firefox", "initialClass": "firefox", "pid": 100, "fullscreen": 0},
                {"address": "0x64e79ee3c460", "mapped": true, "workspace": {"id": 3, "name": "3"}, "floating": true, "monitor": 0, "class": "", "title": "term", "initialClass": "kitty", "pid": 542835, "fullscreen": 2},
                {"address": "0x1", "mapped": false, "workspace": {"id": 1, "name": "1"}, "class": "ghost", "title": "", "monitor": 0},
                {"address": "0x2", "mapped": true, "workspace": {"id": -98, "name": "special:scratch"}, "monitor": 0, "class": "pad", "title": "scratch", "pid": 5}
            ]),
            json!([
                {"id": 0, "name": "eDP-1", "focused": true, "activeWorkspace": {"id": 3, "name": "3"}},
                {"id": 1, "name": "HDMI-A-1", "focused": false, "activeWorkspace": {"id": 4, "name": "4"}}
            ]),
            json!({"address": "0x64e79ee3c460", "class": "kitty"}),
            json!({"keyboards": [
                {"name": "video-bus", "main": false, "active_keymap": "Wrong"},
                {"name": "kb", "main": true, "layout": "us, ru", "active_keymap": "English (US)"}
            ]}),
        )
    }

    #[test]
    fn parses_real_shaped_hyprland_json() {
        let (w, c, m, a, d) = sample();
        let snapshot = parse(&w, &c, &m, &a, &d);
        assert_eq!(snapshot.workspaces.len(), 4);
        let ws = |id: i64| snapshot.workspaces.iter().find(|w| w.id == id).unwrap().clone();
        assert_eq!(
            (ws(3).index, ws(3).active, ws(3).focused, ws(3).has_fullscreen, ws(3).occupied),
            (2, true, true, true, true)
        );
        assert_eq!((ws(4).active, ws(4).focused, ws(4).output.as_str()), (true, false, "HDMI-A-1"));
        assert!(!ws(2).occupied && !ws(2).active);
        assert_eq!(snapshot.focused_output, "eDP-1");
        assert_eq!(snapshot.active_index(), 2);
        assert_eq!(snapshot.active_index_for("HDMI-A-1"), 3);
        assert_eq!(snapshot.windows.len(), 3);
        let term = snapshot.windows.iter().find(|w| w.title == "term").unwrap();
        assert_eq!(term.app_id, "kitty");
        assert!(term.focused && term.floating && term.fullscreen);
        assert_eq!((term.workspace, term.output.as_str(), term.pid), (2, "eDP-1", Some(542835)));
        assert_eq!(term.id, "hypr:0x64e79ee3c460");
        let firefox = snapshot.windows.iter().find(|w| w.app_id == "firefox").unwrap();
        assert!(!firefox.focused && !firefox.fullscreen);
        let special = snapshot.windows.iter().find(|w| w.app_id == "pad").unwrap();
        assert_eq!(special.workspace, -1);
        assert_eq!(snapshot.keyboard_layouts, ["English (US)"]);
        assert_eq!(snapshot.short_keyboard_layout(), "EN");
    }

    #[test]
    fn tolerates_missing_or_empty_pieces() {
        let snapshot = parse(&json!([]), &json!([]), &json!([]), &json!({}), &Value::Null);
        assert_eq!(snapshot, Snapshot::default());
        let snapshot = parse(
            &json!([{"id": 1, "name": "1", "monitor": "X", "windows": 0}]),
            &json!([]),
            &json!([]),
            &json!({}),
            &json!({}),
        );
        assert_eq!(snapshot.workspaces.len(), 1);
        assert!(!snapshot.workspaces[0].active);
    }

    #[test]
    fn socket_paths_come_from_the_environment() {
        let dir = tempfile::tempdir().unwrap();
        let sig = "abc_1_2";
        let hypr = dir.path().join("hypr").join(sig);
        std::fs::create_dir_all(&hypr).unwrap();
        std::fs::write(hypr.join(".socket.sock"), "").unwrap();
        let runtime = dir.path().to_string_lossy().into_owned();
        let env = move |k: &str| match k {
            "HYPRLAND_INSTANCE_SIGNATURE" => Some(sig.to_owned()),
            "XDG_RUNTIME_DIR" => Some(runtime.clone()),
            _ => None,
        };
        let paths = HyprlandPaths::from_env(env).unwrap();
        assert_eq!(paths.requests, hypr.join(".socket.sock"));
        assert_eq!(paths.events, hypr.join(".socket2.sock"));
        assert!(HyprlandPaths::from_env(|_| None).is_none());
    }

    pub struct FakeHyprland {
        pub dir: tempfile::TempDir,
        pub signature: String,
        pub requests: PathBuf,
        pub events: PathBuf,
        pub active: Arc<Mutex<Value>>,
        pub log: Arc<Mutex<Vec<String>>>,
        pub reject_lua: Arc<Mutex<bool>>,
        event_clients: Arc<Mutex<Vec<UnixStream>>>,
    }

    impl FakeHyprland {
        pub fn start() -> FakeHyprland {
            let dir = tempfile::tempdir().unwrap();
            let signature = "sig_1_2".to_owned();
            let socket_dir = dir.path().join("hypr").join(&signature);
            std::fs::create_dir_all(&socket_dir).unwrap();
            let requests = socket_dir.join(".socket.sock");
            let events = socket_dir.join(".socket2.sock");
            let log = Arc::new(Mutex::new(Vec::new()));
            let reject_lua = Arc::new(Mutex::new(false));
            let (w, c, m, a, d) = sample();
            let active = Arc::new(Mutex::new(a));
            let active2 = active.clone();
            let listener = UnixListener::bind(&requests).unwrap();
            let (log2, reject2) = (log.clone(), reject_lua.clone());
            std::thread::spawn(move || {
                for stream in listener.incoming() {
                    let Ok(mut stream) = stream else { break };
                    let mut command = String::new();
                    let mut buffer = [0u8; 4096];
                    let n = stream.read(&mut buffer).unwrap_or(0);
                    command.push_str(&String::from_utf8_lossy(&buffer[..n]));
                    log2.lock().unwrap().push(command.clone());
                    let reply = match command.as_str() {
                        "j/workspaces" => w.to_string(),
                        "j/clients" => c.to_string(),
                        "j/monitors" => m.to_string(),
                        "j/activewindow" => active2.lock().unwrap().to_string(),
                        "j/devices" => d.to_string(),
                        other if other.starts_with("dispatch hl.dsp") && *reject2.lock().unwrap() => {
                            "error: not lua".to_owned()
                        }
                        other
                            if other.starts_with("dispatch")
                                || other.starts_with("switchxkblayout")
                                || other.starts_with("keyword") =>
                        {
                            "ok".to_owned()
                        }
                        _ => "unknown request".to_owned(),
                    };
                    let _ = stream.write_all(reply.as_bytes());
                }
            });
            let event_listener = UnixListener::bind(&events).unwrap();
            let clients: Arc<Mutex<Vec<UnixStream>>> = Arc::default();
            let clients2 = clients.clone();
            std::thread::spawn(move || {
                for stream in event_listener.incoming() {
                    let Ok(stream) = stream else { break };
                    clients2.lock().unwrap().push(stream);
                }
            });
            FakeHyprland { dir, signature, requests, events, active, log, reject_lua, event_clients: clients }
        }

        pub fn push_event(&self, line: &str) {
            let deadline = std::time::Instant::now() + Duration::from_secs(5);
            while self.event_clients.lock().unwrap().is_empty() && std::time::Instant::now() < deadline {
                std::thread::sleep(Duration::from_millis(5));
            }
            for client in self.event_clients.lock().unwrap().iter_mut() {
                let _ = client.write_all(format!("{line}\n").as_bytes());
            }
        }
    }

    #[test]
    fn fetch_runs_all_queries_against_a_server() {
        let fake = FakeHyprland::start();
        let snapshot = fetch(&fake.requests).unwrap();
        assert_eq!(snapshot.windows.len(), 3);
        let log = fake.log.lock().unwrap().clone();
        assert_eq!(log, ["j/workspaces", "j/clients", "j/monitors", "j/activewindow", "j/devices"]);
    }

    #[test]
    fn dispatch_prefers_lua_and_falls_back_to_legacy() {
        let fake = FakeHyprland::start();
        assert!(focus_workspace(&fake.requests, 3).unwrap());
        assert_eq!(fake.log.lock().unwrap().last().unwrap(), "dispatch hl.dsp.focus({ workspace = 3 })");

        *fake.reject_lua.lock().unwrap() = true;
        fake.log.lock().unwrap().clear();
        assert!(focus_window(&fake.requests, "0xabc").unwrap());
        let log = fake.log.lock().unwrap().clone();
        assert_eq!(
            log,
            ["dispatch hl.dsp.focus({ window = \"address:0xabc\" })", "dispatch focuswindow address:0xabc"]
        );

        fake.log.lock().unwrap().clear();
        assert!(close_window(&fake.requests, "0xabc").unwrap());
        assert_eq!(fake.log.lock().unwrap().last().unwrap(), "dispatch closewindow address:0xabc");
        assert!(switch_keyboard_layout(&fake.requests).unwrap());
    }

    #[test]
    fn event_reader_returns_lines_and_times_out() {
        let fake = FakeHyprland::start();
        let mut reader = connect_events(&fake.events, Duration::from_millis(30)).unwrap();
        assert_eq!(reader.next_line().unwrap(), None);
        fake.push_event("workspace>>2");
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        let mut line = None;
        while line.is_none() && std::time::Instant::now() < deadline {
            line = reader.next_line().unwrap();
        }
        assert_eq!(line.as_deref(), Some("workspace>>2"));
    }
}
