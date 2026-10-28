mod blocking;
mod cache_file;
mod clock;
/// Supervised long-running child processes.
pub mod daemon;
mod exec;
/// Periodically refreshed data sources with caching and subscriptions.
pub mod feed;
/// Filesystem helpers that log failures instead of returning them.
pub mod fsx;
mod http;
mod i18n;
mod notify;
/// Helpers deriving values from settings.
pub mod settings;
mod store;
/// Fakes for testing code that uses a [`Ctx`]: a manual scheduler, a fake clock, translator, executor and notifier.
#[cfg(any(test, feature = "testing"))]
pub mod testing;

pub use acanthus_io::{AppId, Dirs};
pub use blocking::Blocking;
pub use cache_file::CacheFile;
pub use clock::{Clock, SystemClock};
pub use daemon::{Daemon, DaemonOptions, DaemonSpawn, DaemonSpec, DaemonState, Restart, spawn_for};
pub use exec::{Cmd, Done, Exec, ProcessExec, Running, program_in_path, run_blocking};
pub use feed::{Feed, FeedSpec, FeedState, FeedStatus, FetchDone, Policy, Subscription};
pub use http::{Http, HttpError, Request, Source, download_argv, download_blocking, get_argv};
pub use i18n::Translator;
pub use notify::{Notice, Notifier, NotifySendNotifier, Urgency, notify_send_argv};
pub use store::{Codec, ConfigStore, DiskStore, KeyedDiskStore, MemoryStore, Store, Stored};

/// The shared facilities a service needs, passed in by the application: process execution, blocking work, HTTP, timers, clock, translation, notifications and directories. Tests substitute fakes.
#[derive(Clone)]
pub struct Ctx {
    /// Runs external programs.
    pub exec: std::rc::Rc<dyn Exec>,
    /// Runs blocking work off the main loop.
    pub blocking: Blocking,
    /// Fetches URLs.
    pub http: Http,
    /// Timer scheduler on the main loop.
    pub schedule: acanthus_io::Scheduler,
    /// Source of the current time.
    pub clock: std::rc::Rc<dyn Clock>,
    /// Looks up translated strings.
    pub i18n: std::rc::Rc<dyn Translator>,
    /// Shows user notifications.
    pub notify: std::rc::Rc<dyn Notifier>,
    /// Directories derived from the application identity.
    pub dirs: Dirs,
}
