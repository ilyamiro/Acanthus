//! I/O toolkit of Acanthus on a calloop event loop: child processes, file watching, timers, atomic file writes, per-app directories and a JSON line IPC socket.
#![deny(missing_docs)]

mod dirs;
mod error;
mod fs;
mod ipc;
mod process;
/// Timers on the event loop.
pub mod timer;
mod watch;
mod weak;

pub use calloop::LoopHandle;
pub use dirs::{AppId, Dirs};
pub use error::IoError;
pub use fs::{atomic_write, read_to_string_lossy};
pub use ipc::{IpcClient, IpcRegistry, IpcServer};
pub use process::{ExitInfo, Process, ProcessHandle, exec_detached, run_to_completion};
pub use timer::{Scheduler, TimerHandle};
pub use watch::{Watch, WatchEvent};
pub use weak::WeakHandle;
