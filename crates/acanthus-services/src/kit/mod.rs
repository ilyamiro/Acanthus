mod blocking;
mod cache_file;
mod clock;
pub mod daemon;
mod exec;
pub mod feed;
pub mod fsx;
mod http;
mod i18n;
mod notify;
pub mod settings;
mod store;
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

#[derive(Clone)]
pub struct Ctx {
    pub exec: std::rc::Rc<dyn Exec>,
    pub blocking: Blocking,
    pub http: Http,
    pub schedule: acanthus_io::Scheduler,
    pub clock: std::rc::Rc<dyn Clock>,
    pub i18n: std::rc::Rc<dyn Translator>,
    pub notify: std::rc::Rc<dyn Notifier>,
    pub dirs: Dirs,
}
