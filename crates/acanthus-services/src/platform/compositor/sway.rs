use std::io::{Read, Write};
use std::os::unix::net::UnixStream;
use std::path::Path;
use std::time::Duration;

use serde_json::Value;

use super::types::{Snapshot, Window, Workspace};

const MAGIC: &[u8; 6] = b"i3-ipc";
pub const RUN_COMMAND: u32 = 0;
pub const GET_WORKSPACES: u32 = 1;
pub const SUBSCRIBE: u32 = 2;
pub const GET_TREE: u32 = 4;
pub const GET_INPUTS: u32 = 100;
pub const EVENT_FLAG: u32 = 0x8000_0000;

pub fn encode_frame(kind: u32, payload: &[u8]) -> Vec<u8> {
    let mut frame = Vec::with_capacity(14 + payload.len());
    frame.extend_from_slice(MAGIC);
    frame.extend_from_slice(&(payload.len() as u32).to_ne_bytes());
    frame.extend_from_slice(&kind.to_ne_bytes());
    frame.extend_from_slice(payload);
    frame
}

fn read_exact_patient(stream: &mut UnixStream, buffer: &mut [u8], allow_first_timeout: bool) -> std::io::Result<bool> {
    let mut filled = 0;
    let mut stalls = 0;
    while filled < buffer.len() {
        match stream.read(&mut buffer[filled..]) {
            Ok(0) => return Err(std::io::Error::from(std::io::ErrorKind::UnexpectedEof)),
            Ok(n) => {
                filled += n;
                stalls = 0;
            }
            Err(e) if matches!(e.kind(), std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut) => {
                if filled == 0 && allow_first_timeout {
                    return Ok(false);
                }
                stalls += 1;
                if stalls > 100 {
                    return Err(e);
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
            Err(e) => return Err(e),
        }
    }
    Ok(true)
}

pub fn read_frame(stream: &mut UnixStream) -> std::io::Result<Option<(u32, Vec<u8>)>> {
    let mut header = [0u8; 14];
    if !read_exact_patient(stream, &mut header, true)? {
        return Ok(None);
    }
    if &header[..6] != MAGIC {
        return Err(std::io::Error::new(std::io::ErrorKind::InvalidData, "bad i3-ipc magic"));
    }
    let length = u32::from_ne_bytes(header[6..10].try_into().expect("4 bytes")) as usize;
    let kind = u32::from_ne_bytes(header[10..14].try_into().expect("4 bytes"));
    let mut payload = vec![0u8; length];
    read_exact_patient(stream, &mut payload, false)?;
    Ok(Some((kind, payload)))
}

pub struct SwayConnection {
    stream: UnixStream,
}

impl SwayConnection {
    pub fn connect(socket: &Path, poll: Duration) -> std::io::Result<SwayConnection> {
        let stream = UnixStream::connect(socket)?;
        stream.set_read_timeout(Some(poll))?;
        stream.set_write_timeout(Some(Duration::from_secs(2)))?;
        Ok(SwayConnection { stream })
    }

    pub fn call(&mut self, kind: u32, payload: &str) -> std::io::Result<Value> {
        self.stream.write_all(&encode_frame(kind, payload.as_bytes()))?;
        let deadline = std::time::Instant::now() + Duration::from_secs(3);
        loop {
            match read_frame(&mut self.stream)? {
                Some((reply_kind, body)) if reply_kind == kind => {
                    return serde_json::from_slice(&body)
                        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e));
                }
                Some(_) => {}
                None if std::time::Instant::now() > deadline => {
                    return Err(std::io::Error::from(std::io::ErrorKind::TimedOut));
                }
                None => {}
            }
        }
    }

    pub fn subscribe(&mut self, events: &[&str]) -> std::io::Result<bool> {
        let reply = self.call(SUBSCRIBE, &serde_json::to_string(events).unwrap_or_default())?;
        Ok(reply.get("success").and_then(Value::as_bool) == Some(true))
    }

    pub fn next_event(&mut self) -> std::io::Result<Option<(u32, Value)>> {
        match read_frame(&mut self.stream)? {
            Some((kind, body)) if kind & EVENT_FLAG != 0 => {
                Ok(Some((kind, serde_json::from_slice(&body).unwrap_or(Value::Null))))
            }
            _ => Ok(None),
        }
    }

    pub fn fetch(&mut self) -> std::io::Result<Snapshot> {
        let workspaces = self.call(GET_WORKSPACES, "")?;
        let tree = self.call(GET_TREE, "")?;
        let inputs = self.call(GET_INPUTS, "").unwrap_or(Value::Null);
        Ok(parse(&workspaces, &tree, &inputs))
    }

    pub fn run(&mut self, command: &str) -> std::io::Result<bool> {
        let reply = self.call(RUN_COMMAND, command)?;
        Ok(reply.as_array().is_some_and(|r| r.iter().all(|c| c.get("success").and_then(Value::as_bool) == Some(true))))
    }
}

fn workspace_index(node: &Value) -> i32 {
    let name = node.get("name").and_then(Value::as_str).unwrap_or("");
    let num = node
        .get("num")
        .and_then(Value::as_i64)
        .filter(|n| *n > 0)
        .or_else(|| name.parse::<i64>().ok().filter(|n| *n > 0));
    num.map_or(-1, |n| (n - 1) as i32)
}

fn collect_windows(node: &Value, output: &str, workspace: i32, workspace_name: &str, out: &mut Vec<Window>) {
    let mut output = output.to_owned();
    let mut workspace = workspace;
    let mut workspace_name = workspace_name.to_owned();
    match node.get("type").and_then(Value::as_str) {
        Some("output") => {
            if let Some(name) = node.get("name").and_then(Value::as_str) {
                output = name.to_owned();
            }
        }
        Some("workspace") => {
            workspace_name = node.get("name").and_then(Value::as_str).unwrap_or("").to_owned();
            workspace = workspace_index(node);
        }
        _ => {}
    }

    let kind = node.get("type").and_then(Value::as_str);
    let app_id =
        node.get("app_id").and_then(Value::as_str).filter(|s| !s.is_empty()).map(str::to_owned).or_else(|| {
            node.get("window_properties")
                .and_then(|p| p.get("class"))
                .and_then(Value::as_str)
                .filter(|s| !s.is_empty())
                .map(str::to_owned)
        });
    if matches!(kind, Some("con" | "floating_con"))
        && let Some(app_id) = app_id
    {
        let id = node.get("id").and_then(Value::as_i64).unwrap_or(0);
        out.push(Window {
            id: format!("sway:{id}"),
            native_id: id.to_string(),
            app_id,
            title: node.get("name").and_then(Value::as_str).unwrap_or("").to_owned(),
            workspace,
            workspace_name: workspace_name.clone(),
            output: output.clone(),
            focused: node.get("focused").and_then(Value::as_bool).unwrap_or(false),
            fullscreen: node.get("fullscreen_mode").and_then(Value::as_i64).unwrap_or(0) > 0,
            floating: kind == Some("floating_con"),
            urgent: node.get("urgent").and_then(Value::as_bool).unwrap_or(false),
            pid: node.get("pid").and_then(Value::as_i64),
        });
    }

    for key in ["nodes", "floating_nodes"] {
        for child in node.get(key).and_then(Value::as_array).into_iter().flatten() {
            collect_windows(child, &output, workspace, &workspace_name, out);
        }
    }
}

pub fn parse(workspaces: &Value, tree: &Value, inputs: &Value) -> Snapshot {
    let mut windows = Vec::new();
    collect_windows(tree, "", -1, "", &mut windows);

    let mut focused_output = String::new();
    let workspace_list: Vec<Workspace> = workspaces
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|w| {
            let index = workspace_index(w);
            if index < 0 {
                return None;
            }
            let flag = |key: &str| w.get(key).and_then(Value::as_bool).unwrap_or(false);
            let output = w.get("output").and_then(Value::as_str).unwrap_or("").to_owned();
            if flag("focused") {
                focused_output = output.clone();
            }
            let name = w.get("name").and_then(Value::as_str).unwrap_or("").to_owned();
            Some(Workspace {
                id: w.get("id").and_then(Value::as_i64).unwrap_or(i64::from(index) + 1),
                index,
                occupied: windows.iter().any(|win| win.workspace == index && win.output == output),
                name,
                output,
                active: flag("visible"),
                focused: flag("focused"),
                urgent: flag("urgent"),
                has_fullscreen: false,
            })
        })
        .collect();

    let keyboard = inputs
        .as_array()
        .into_iter()
        .flatten()
        .find(|i| i.get("type").and_then(Value::as_str) == Some("keyboard") && i.get("xkb_layout_names").is_some());
    let layouts = keyboard
        .and_then(|k| k.get("xkb_layout_names"))
        .and_then(Value::as_array)
        .map(|names| names.iter().filter_map(|n| n.as_str().map(str::to_owned)).collect())
        .unwrap_or_default();
    let layout_index =
        keyboard.and_then(|k| k.get("xkb_active_layout_index")).and_then(Value::as_u64).unwrap_or(0) as usize;

    Snapshot {
        workspaces: workspace_list,
        windows,
        focused_output,
        keyboard_layouts: layouts,
        keyboard_layout_index: layout_index,
    }
}

#[cfg(test)]
pub mod tests {
    use std::os::unix::net::UnixListener;

    use serde_json::json;

    use super::*;

    pub fn sample_workspaces() -> Value {
        json!([
            {"id": 1, "num": 1, "name": "1", "output": "eDP-1", "visible": true, "focused": true, "urgent": false},
            {"id": 2, "num": 2, "name": "2", "output": "eDP-1", "visible": false, "focused": false, "urgent": true},
            {"id": 3, "num": -1, "name": "scratch", "output": "eDP-1", "visible": false, "focused": false, "urgent": false}
        ])
    }

    pub fn sample_tree() -> Value {
        json!({"type": "root", "nodes": [
            {"type": "output", "name": "eDP-1", "nodes": [
                {"type": "workspace", "name": "1", "num": 1, "nodes": [
                    {"type": "con", "id": 10, "app_id": "foot", "name": "shell", "focused": true, "pid": 77, "nodes": []},
                    {"type": "con", "id": 11, "name": "x11 app", "window_properties": {"class": "Steam"}, "nodes": []},
                    {"type": "con", "id": 12, "name": "split container", "nodes": [
                        {"type": "con", "id": 13, "app_id": "firefox", "name": "web", "fullscreen_mode": 1, "nodes": []}
                    ]}
                ], "floating_nodes": [
                    {"type": "floating_con", "id": 14, "app_id": "pavucontrol", "name": "volume", "nodes": []}
                ]},
                {"type": "workspace", "name": "2", "num": 2, "nodes": []}
            ]}
        ]})
    }

    #[test]
    fn frames_round_trip_with_the_i3_ipc_header() {
        let frame = encode_frame(GET_TREE, b"{}");
        assert_eq!(&frame[..6], b"i3-ipc");
        assert_eq!(u32::from_ne_bytes(frame[6..10].try_into().unwrap()), 2);
        assert_eq!(u32::from_ne_bytes(frame[10..14].try_into().unwrap()), GET_TREE);
        assert_eq!(&frame[14..], b"{}");
    }

    #[test]
    fn parses_tree_workspaces_and_inputs() {
        let inputs = json!([
            {"type": "pointer", "identifier": "mouse"},
            {"type": "keyboard", "xkb_layout_names": ["English (US)", "Russian"], "xkb_active_layout_index": 1}
        ]);
        let snapshot = parse(&sample_workspaces(), &sample_tree(), &inputs);
        assert_eq!(snapshot.workspaces.len(), 2);
        assert_eq!(snapshot.windows.len(), 5 - 1);
        let ws1 = &snapshot.workspaces[0];
        assert!(ws1.active && ws1.focused && ws1.occupied);
        let ws2 = &snapshot.workspaces[1];
        assert!(ws2.urgent && !ws2.occupied && !ws2.active);
        assert_eq!(snapshot.focused_output, "eDP-1");
        let steam = snapshot.windows.iter().find(|w| w.native_id == "11").unwrap();
        assert_eq!(steam.app_id, "Steam");
        let firefox = snapshot.windows.iter().find(|w| w.app_id == "firefox").unwrap();
        assert!(firefox.fullscreen);
        assert_eq!((firefox.workspace, firefox.output.as_str(), firefox.workspace_name.as_str()), (0, "eDP-1", "1"));
        let floating = snapshot.windows.iter().find(|w| w.app_id == "pavucontrol").unwrap();
        assert!(floating.floating);
        assert!(snapshot.windows.iter().find(|w| w.native_id == "10").unwrap().focused);
        assert_eq!(snapshot.keyboard_layout(), "Russian");
    }

    #[test]
    fn named_workspaces_without_numbers_are_skipped() {
        let snapshot = parse(
            &json!([{"id": 9, "num": -1, "name": "web", "output": "A", "visible": true, "focused": true}]),
            &json!({}),
            &json!([]),
        );
        assert!(snapshot.workspaces.is_empty());
    }

    pub fn serve(path: std::path::PathBuf) {
        let listener = UnixListener::bind(path).unwrap();
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { break };
                std::thread::spawn(move || {
                    loop {
                        let mut header = [0u8; 14];
                        if stream.read_exact(&mut header).is_err() {
                            return;
                        }
                        let length = u32::from_ne_bytes(header[6..10].try_into().unwrap()) as usize;
                        let kind = u32::from_ne_bytes(header[10..14].try_into().unwrap());
                        let mut payload = vec![0u8; length];
                        let _ = stream.read_exact(&mut payload);
                        let reply = match kind {
                            GET_WORKSPACES => sample_workspaces(),
                            GET_TREE => sample_tree(),
                            GET_INPUTS => json!([]),
                            SUBSCRIBE => json!({"success": true}),
                            RUN_COMMAND => json!([{"success": String::from_utf8_lossy(&payload).contains("focus")}]),
                            _ => json!({}),
                        };
                        let _ = stream.write_all(&encode_frame(kind, reply.to_string().as_bytes()));
                        if kind == SUBSCRIBE {
                            std::thread::sleep(Duration::from_millis(20));
                            let _ = stream.write_all(&encode_frame(EVENT_FLAG | 3, br#"{"change":"focus"}"#));
                        }
                    }
                });
            }
        });
    }

    #[test]
    fn talks_to_a_server_fetches_runs_commands_and_receives_events() {
        let dir = tempfile::tempdir().unwrap();
        let socket = dir.path().join("sway.sock");
        serve(socket.clone());
        let mut connection = SwayConnection::connect(&socket, Duration::from_millis(50)).unwrap();
        let snapshot = connection.fetch().unwrap();
        assert_eq!(snapshot.windows.len(), 4);
        assert!(connection.run("[con_id=10] focus").unwrap());
        assert!(!connection.run("nonsense").unwrap());

        let mut events = SwayConnection::connect(&socket, Duration::from_millis(50)).unwrap();
        assert!(events.subscribe(&["window", "workspace"]).unwrap());
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        let mut event = None;
        while event.is_none() && std::time::Instant::now() < deadline {
            event = events.next_event().unwrap();
        }
        let (kind, body) = event.expect("event received");
        assert_eq!(kind, EVENT_FLAG | 3);
        assert_eq!(body["change"], "focus");
    }
}
