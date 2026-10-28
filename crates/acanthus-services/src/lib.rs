pub mod kit;
pub mod platform;

pub mod prelude {
    pub use crate::kit::{Clock, Ctx, Notice, Notifier, Translator};
    pub use crate::platform::compositor::Compositor;
    pub use crate::platform::desktop_service::DesktopEntries;
    pub use crate::platform::mpris::Mpris;
    #[cfg(feature = "pipewire")]
    pub use crate::platform::pipewire::Pipewire;
    pub use crate::platform::toplevels::Toplevels;
    pub use crate::platform::upower::UPower;
}
