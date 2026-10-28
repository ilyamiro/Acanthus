# acanthus-io

**Parent context:** `../../AGENTS.md`
**Generated:** 2026-10-22 · **Updated:** 2026-10-22

## Purpose
calloop-driven I/O primitives. Everything registers sources on a `LoopHandle<'static, D>` and never blocks the UI thread.

## Key Files
| File | Description |
|------|-------------|
| `src/process.rs` | `Process` builder (pidfd exit, line callbacks, `collect_output`, kill on drop), `ProcessHandle`, `exec_detached`, `run_to_completion` |
| `src/watch.rs` | `Watch::file` / `Watch::directory` over inotify (rustix); watches the parent dir so atomic replaces are seen |
| `src/fs.rs` | `atomic_write` (tmp + fsync + rename, 0644), `read_to_string_lossy` |
| `src/timer.rs` | `after`, `every`, `dynamic` timers returning a cancel-on-drop `TimerHandle` |
| `src/ipc.rs` | `IpcRegistry` ((target, method) handlers), `IpcServer` (Unix socket, one JSON object per line), `IpcClient::call` |
| `src/weak.rs` | `WeakHandle`: weak loop handle whose `remove` defers re-entrant calls |

## For AI Agents

### Working In This Directory
- Inside source closures hold a `WeakHandle`, never a strong `LoopHandle`, and remove sources only through `WeakHandle::remove`. calloop panics on re-entrant removal and a strong handle creates a reference cycle.
- IPC wire format: request `{"target","method","args":[..]}`, reply `{"ok":true,"value":..}` or `{"ok":false,"error":".."}`. Target and method names must match the QML `IpcHandler`s exactly.

### Testing Requirements
`cargo test -p acanthus-io` (19 tests, real calloop loops and tempdirs). Do not assume absence of inotify events (forked fd copies delay `CLOSE_WRITE`).

## Dependencies
Internal: acanthus-core. External: calloop, rustix, serde_json.
