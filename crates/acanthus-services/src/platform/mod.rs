/// Maps window classes and application ids to desktop entries.
pub mod app_matcher;
/// Bluetooth devices, pairing and control through BlueZ.
#[cfg(feature = "bluetooth")]
pub mod bluetooth;
/// Bridges between worker threads and the main loop: event sinks, command handles, stop flags and supervised workers.
pub mod bridge;
/// Screen brightness through backlight devices.
#[cfg(feature = "brightness")]
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
#[cfg(feature = "file-chooser")]
pub mod file_chooser;
/// Colour temperature through the wlroots gamma protocol.
#[cfg(feature = "gamma")]
pub mod gamma;
/// Idle detection through the Wayland idle-notify protocol.
#[cfg(feature = "idle")]
pub mod idle;
/// Media players over MPRIS.
#[cfg(feature = "mpris")]
pub mod mpris;
/// Network state and control through NetworkManager.
#[cfg(feature = "network")]
pub mod network;
/// Import of VPN and Wi-Fi configurations into NetworkManager.
#[cfg(feature = "network")]
pub mod network_import;
/// The notification server and its history.
#[cfg(feature = "notifications")]
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
/// Desktop preferences from the XDG Settings portal: reduced motion.
#[cfg(feature = "portal-settings")]
pub mod portal_settings;
/// Power profiles through power-profiles-daemon.
#[cfg(feature = "power-profiles")]
pub mod power_profiles;
/// Screen saver and idle inhibition over D-Bus.
#[cfg(feature = "screensaver")]
pub mod screensaver;
/// Global keyboard shortcuts through compositor bindings or the XDG GlobalShortcuts portal.
#[cfg(feature = "shortcuts")]
pub mod shortcuts;
/// Foreign toplevel windows from the compositor.
#[cfg(feature = "toplevels")]
pub mod toplevels;
/// System tray items over StatusNotifierItem and dbusmenu.
#[cfg(feature = "tray")]
pub mod tray;
/// Battery and power state through UPower.
#[cfg(feature = "upower")]
pub mod upower;
