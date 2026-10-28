# Changelog

All notable changes to the Acanthus engine crates. The policy is in `docs/engine/versioning.md`.

## Unreleased

### Added

- `acanthus_wayland::{SystemClipboard, Selection}` and `App::clipboard`: the desktop clipboard through `ext-data-control-v1` or `zwlr_data_control_v1`; `acanthus_scene::{ClipboardBackend, Clipboard::set_backend, Scene::set_clipboard_backend}` route text widgets through it.
- `acanthus_wayland::{LockConfig, LockPhase}`, `App::lock` and `AppHandle::unlock`: `ext-session-lock-v1` with a lock surface per output and a signal of the lock phase.
- `acanthus_wayland::{WindowConfig, WindowState, WindowCommand, Decorations, ResizeEdge}` and `App::window`: xdg_toplevel windows with title, app id, size limits, close requests and interactive move and resize.
- Engine crates `acanthus-core`, `acanthus-anim`, `acanthus-color`, `acanthus-image`, `acanthus-scene`, `acanthus-render`, `acanthus-wayland`, `acanthus-io`, `acanthus-runtime`, `acanthus-services`, `acanthus-widgets` and `acanthus-app`, version `0.1.0`, not released yet.
- `Node::animate(Prop, Motion)`: QML `Behavior on <property>` for position, size, opacity, scale, rotation, colour, radius and border.
- `Tokens` (motion, spacing, radii) on `Theme`, `SettingKeys::reduced_motion` and reduced motion in `acanthus-anim` (`set_reduced_motion`, `reduced_motion`, `FAST`/`BASE`/`SLOW`).
- `cargo xtask api [--check]` and the per-crate public API lists in `docs/api/`, checked by `scripts/check.sh`.
- `PropSpec::themed`, `Registry::with_colors` and `Theme::role`: colour props follow theme roles when not given.
- `PortalSettings` (feature `portal-settings`) and `Theme::reduced_motion_system`: the desktop's animation preference.
- `acanthus_app::testing`: `Headless` (offscreen shots, scripted pointer and key replay on a virtual 60 Hz clock, stall injection, JSON frame log), `Script`, `Image`, `Run`.
- `acanthus_widgets::menu` (`menu`, `MenuItem`, `MenuEntry`, `Outcome`, keyboard helpers) and `acanthus_app::menu` (`context_popup`, `submenu_popup`).
- Examples `hello-bar`, `hello-popup` and `hello-widget`.
