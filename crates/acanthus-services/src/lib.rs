//! Native system services as reactive signals, plus the kit they are built on.

#![deny(missing_docs)]

/// Platform-independent building blocks for services: process execution, HTTP, timers, stores, feeds, daemons, notifications and the context that carries them.
pub mod kit;
/// Services that talk to the system: compositors, audio, network, Bluetooth, power, notifications, tray, authentication and more, each exposing its state as signals.
pub mod platform;

/// The service types most applications import together.
pub mod prelude {
    pub use crate::kit::{Clock, Ctx, Notice, Notifier, Translator};
    pub use crate::platform::compositor::Compositor;
    pub use crate::platform::desktop_service::DesktopEntries;
    #[cfg(feature = "mpris")]
    pub use crate::platform::mpris::Mpris;
    #[cfg(feature = "pipewire")]
    pub use crate::platform::pipewire::Pipewire;
    #[cfg(feature = "toplevels")]
    pub use crate::platform::toplevels::Toplevels;
    #[cfg(feature = "upower")]
    pub use crate::platform::upower::UPower;
}
