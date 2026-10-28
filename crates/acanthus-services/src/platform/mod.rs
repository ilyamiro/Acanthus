/// Maps window classes and application ids to desktop entries.
pub mod app_matcher;
/// Bluetooth devices, pairing and control through BlueZ.
pub mod bluetooth;
/// Bridges between worker threads and the main loop: event sinks, command handles, stop flags and supervised workers.
pub mod bridge;
/// Screen brightness through backlight devices.
pub mod brightness;
/// Compositor-neutral windows, workspaces, outputs and keyboard layout, with drivers per compositor.
pub mod compositor;
/// D-Bus connection helpers and value conversion.
pub mod dbus;
/// Parsing of desktop entries and the application directories.
pub mod desktop;
/// The desktop entry service: the installed applications, kept up to date.
pub mod desktop_service;
/// File choosing through the XDG desktop portal.
pub mod file_chooser;
/// Colour temperature through the wlroots gamma protocol.
pub mod gamma;
/// Idle detection through the Wayland idle-notify protocol.
pub mod idle;
/// Media players over MPRIS.
pub mod mpris;
/// Network state and control through NetworkManager.
pub mod network;
/// Import of VPN and Wi-Fi configurations into NetworkManager.
pub mod network_import;
/// The notification server and its history.
pub mod notifications;
/// User authentication through PAM.
#[cfg(feature = "pam")]
pub mod pam;
/// PAM configuration helpers, including the fingerprint-free variant.
#[cfg(feature = "pam")]
pub mod pam_config;
/// Audio devices, streams and levels through PipeWire.
#[cfg(feature = "pipewire")]
pub mod pipewire;
/// A polkit authentication agent.
#[cfg(feature = "polkit")]
pub mod polkit;
/// Power profiles through power-profiles-daemon.
pub mod power_profiles;
/// Screen saver and idle inhibition over D-Bus.
pub mod screensaver;
/// Foreign toplevel windows from the compositor.
pub mod toplevels;
/// System tray items over StatusNotifierItem and dbusmenu.
pub mod tray;
/// Battery and power state through UPower.
pub mod upower;
