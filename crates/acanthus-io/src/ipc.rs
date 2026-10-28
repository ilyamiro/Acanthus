use std::cell::RefCell;
use std::collections::BTreeMap;
use std::io::{BufRead, BufReader, Write};
use std::os::fd::OwnedFd;
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::Duration;

use calloop::generic::Generic;
use calloop::{Interest, LoopHandle, Mode, PostAction, RegistrationToken};
use serde_json::{Value, json};

use crate::error::IoError;
use crate::weak::WeakHandle;

type Handler = Rc<RefCell<dyn FnMut(&[Value]) -> Result<Value, String>>>;

#[derive(Clone, Default)]
/// Table of IPC functions addressed by target and method name.
pub struct IpcRegistry {
    handlers: Rc<RefCell<BTreeMap<(String, String), Handler>>>,
}

impl IpcRegistry {
    /// Creates an empty registry.
    pub fn new() -> Self {
        Self::default()
    }

    /// Registers `handler` for `target.method`, replacing any previous one.
    pub fn register(
        &self,
        target: &str,
        method: &str,
        handler: impl FnMut(&[Value]) -> Result<Value, String> + 'static,
    ) {
        self.handlers.borrow_mut().insert((target.to_owned(), method.to_owned()), Rc::new(RefCell::new(handler)));
    }

    /// Removes every function of `target`.
    pub fn unregister_target(&self, target: &str) {
        self.handlers.borrow_mut().retain(|(t, _), _| t != target);
    }

    /// All registered `(target, method)` pairs.
    pub fn methods(&self) -> Vec<(String, String)> {
        self.handlers.borrow().keys().cloned().collect()
    }

    /// Calls a function directly. Errors for unknown functions and for re-entrant calls.
    pub fn call(&self, target: &str, method: &str, args: &[Value]) -> Result<Value, String> {
        let handler = self
            .handlers
            .borrow()
            .get(&(target.to_owned(), method.to_owned()))
            .cloned()
            .ok_or_else(|| format!("unknown ipc function {target}.{method}"))?;
        let mut handler = handler.try_borrow_mut().map_err(|_| format!("{target}.{method} is already running"))?;
        (*handler)(args)
    }

    /// Handles one JSON request line (`target`, `method`, `args`) and returns the JSON response with an `ok` field.
    pub fn dispatch_line(&self, line: &str) -> Value {
        let request: Value = match serde_json::from_str(line) {
            Ok(v) => v,
            Err(e) => return json!({"ok": false, "error": format!("invalid request: {e}")}),
        };
        let field = |name: &str| request.get(name).and_then(Value::as_str);
        let (Some(target), Some(method)) = (field("target"), field("method")) else {
            return json!({"ok": false, "error": "request needs target and method"});
        };
        let args = request.get("args").and_then(Value::as_array).cloned().unwrap_or_default();
        match self.call(target, method, &args) {
            Ok(value) => json!({"ok": true, "value": value}),
            Err(error) => json!({"ok": false, "error": error}),
        }
    }
}

/// Unix socket server answering line-delimited JSON requests from an [`IpcRegistry`]. Dropping it removes the socket.
pub struct IpcServer {
    path: PathBuf,
    remove: Option<Box<dyn FnOnce()>>,
}

impl IpcServer {
    /// Binds `path` on the event loop, replacing a stale socket and refusing one that is in use.
    pub fn bind<D: 'static>(
        handle: &LoopHandle<'static, D>,
        path: &Path,
        registry: IpcRegistry,
    ) -> Result<IpcServer, IoError> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        if path.exists() {
            if UnixStream::connect(path).is_ok() {
                return Err(IoError::Ipc(format!("another instance is listening on {}", path.display())));
            }
            std::fs::remove_file(path)?;
        }
        let listener = UnixListener::bind(path)?;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
        listener.set_nonblocking(true)?;
        let clients: Rc<RefCell<Vec<RegistrationToken>>> = Rc::default();

        let accept_handle = WeakHandle::new(handle);
        let accept_clients = clients.clone();
        let listener_token = handle.insert_source(
            Generic::new(OwnedFd::from(listener.try_clone()?), Interest::READ, Mode::Level),
            move |_, _, _| {
                let Some(handle) = accept_handle.upgrade() else { return Ok(PostAction::Remove) };
                while let Ok((stream, _)) = listener.accept() {
                    if let Err(e) = add_client(&handle, &accept_clients, stream, registry.clone()) {
                        tracing::warn!("ipc client rejected: {e}");
                    }
                }
                Ok(PostAction::Continue)
            },
        )?;

        let remove_handle = WeakHandle::new(handle);
        let remove_path = path.to_path_buf();
        Ok(IpcServer {
            path: path.to_path_buf(),
            remove: Some(Box::new(move || {
                remove_handle.remove(listener_token);
                let tokens: Vec<RegistrationToken> = clients.borrow_mut().drain(..).collect();
                for token in tokens {
                    remove_handle.remove(token);
                }
                let _ = std::fs::remove_file(remove_path);
            })),
        })
    }

    /// The socket path.
    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for IpcServer {
    fn drop(&mut self) {
        if let Some(remove) = self.remove.take() {
            remove();
        }
    }
}

fn add_client<D: 'static>(
    handle: &LoopHandle<'static, D>,
    clients: &Rc<RefCell<Vec<RegistrationToken>>>,
    stream: UnixStream,
    registry: IpcRegistry,
) -> Result<(), IoError> {
    stream.set_nonblocking(true)?;
    let writer = stream.try_clone()?;
    let mut pending: Vec<u8> = Vec::new();
    let token =
        handle.insert_source(Generic::new(OwnedFd::from(stream), Interest::READ, Mode::Level), move |_, fd, _| {
            let mut chunk = [0u8; 4096];
            let mut closed = false;
            loop {
                match rustix::io::read(&**fd, &mut chunk) {
                    Ok(0) => {
                        closed = true;
                        break;
                    }
                    Ok(n) => pending.extend_from_slice(&chunk[..n]),
                    Err(rustix::io::Errno::INTR) => continue,
                    Err(rustix::io::Errno::AGAIN) => break,
                    Err(_) => {
                        closed = true;
                        break;
                    }
                }
            }
            while let Some(i) = pending.iter().position(|b| *b == b'\n') {
                let line: Vec<u8> = pending.drain(..=i).collect();
                let reply = registry.dispatch_line(String::from_utf8_lossy(&line).trim());
                let mut text = reply.to_string();
                text.push('\n');
                write_all_retry(&writer, text.as_bytes());
            }
            Ok(if closed { PostAction::Remove } else { PostAction::Continue })
        })?;
    clients.borrow_mut().push(token);
    Ok(())
}

fn write_all_retry(mut stream: &UnixStream, mut bytes: &[u8]) {
    let mut stalls = 0;
    while !bytes.is_empty() {
        match stream.write(bytes) {
            Ok(0) => return,
            Ok(n) => bytes = &bytes[n..],
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock && stalls < 200 => {
                stalls += 1;
                std::thread::sleep(Duration::from_millis(1));
            }
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
            Err(_) => return,
        }
    }
}

/// Client for an [`IpcServer`].
pub struct IpcClient;

impl IpcClient {
    /// Sends one request and returns the response, with a 5 second read timeout.
    pub fn call(path: &Path, target: &str, method: &str, args: &[Value]) -> Result<Value, IoError> {
        let mut stream = UnixStream::connect(path)?;
        stream.set_read_timeout(Some(Duration::from_secs(5)))?;
        let mut request = json!({"target": target, "method": method, "args": args}).to_string();
        request.push('\n');
        stream.write_all(request.as_bytes())?;
        let mut line = String::new();
        BufReader::new(&stream).read_line(&mut line)?;
        let reply: Value = serde_json::from_str(line.trim())?;
        if reply.get("ok").and_then(Value::as_bool) == Some(true) {
            Ok(reply.get("value").cloned().unwrap_or(Value::Null))
        } else {
            Err(IoError::Ipc(reply.get("error").and_then(Value::as_str).unwrap_or("unknown error").to_owned()))
        }
    }
}

#[cfg(test)]
mod tests {
    use std::time::Instant;

    use calloop::EventLoop;

    use super::*;

    #[test]
    fn registry_dispatches_and_reports_errors() {
        let registry = IpcRegistry::new();
        registry.register("osd", "show", |args| Ok(json!(args.len())));
        registry.register("lock", "fail", |_| Err("nope".into()));
        assert_eq!(registry.dispatch_line(r#"{"target":"osd","method":"show","args":[1,2]}"#)["value"], 2);
        assert_eq!(registry.dispatch_line(r#"{"target":"lock","method":"fail"}"#)["error"], "nope");
        assert_eq!(registry.dispatch_line(r#"{"target":"x","method":"y"}"#)["ok"], false);
        assert_eq!(registry.dispatch_line("not json")["ok"], false);
        assert_eq!(registry.dispatch_line(r#"{"method":"y"}"#)["ok"], false);
    }

    #[test]
    fn handlers_may_register_more_handlers() {
        let registry = IpcRegistry::new();
        let inner = registry.clone();
        registry.register("a", "go", move |_| {
            inner.register("b", "late", |_| Ok(json!("ok")));
            Ok(Value::Null)
        });
        registry.call("a", "go", &[]).unwrap();
        assert_eq!(registry.call("b", "late", &[]).unwrap(), json!("ok"));
    }

    #[test]
    fn socket_round_trip_and_cleanup() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("run/ipc.sock");
        let mut event_loop: EventLoop<'static, ()> = EventLoop::try_new().unwrap();
        let registry = IpcRegistry::new();
        registry.register("main", "echo", |args| Ok(args.first().cloned().unwrap_or(Value::Null)));
        let server = IpcServer::bind(&event_loop.handle(), &path, registry.clone()).unwrap();

        let client_path = path.clone();
        let worker = std::thread::spawn(move || IpcClient::call(&client_path, "main", "echo", &[json!("hi")]));
        let deadline = Instant::now() + Duration::from_secs(5);
        while !worker.is_finished() && Instant::now() < deadline {
            event_loop.dispatch(Duration::from_millis(10), &mut ()).unwrap();
        }
        assert_eq!(worker.join().unwrap().unwrap(), json!("hi"));

        let second = IpcServer::bind(&event_loop.handle(), &path, registry);
        assert!(second.is_err());
        drop(server);
        assert!(!path.exists());
    }

    #[test]
    fn stale_socket_is_replaced() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("ipc.sock");
        std::fs::write(&path, "").unwrap();
        assert!(path.exists());
        let event_loop: EventLoop<'static, ()> = EventLoop::try_new().unwrap();
        let _server = IpcServer::bind(&event_loop.handle(), &path, IpcRegistry::new()).unwrap();
    }
}
