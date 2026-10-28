# acanthus-services

**Parent context:** `../../AGENTS.md`
**Generated:** 2026-10-22 · **Updated:** 2026-10-23

## Purpose
System integration as reactive signals. Blocking or async work runs on worker threads and is delivered to the UI thread through calloop channels, where it is applied to `Signal`s (signals are `!Send`).

## Layers
Dependency rule: `kit` depends on nothing else in the crate; `platform` depends only on `kit`. Application feeds and controllers live in the application. The crate does not depend on `acanthus-runtime`; the application adapts `Runtime` into a `kit::Ctx`.

| Module | What lives there |
|--------|------------------|
| `src/kit/` | the general internal service mechanism: `Ctx`, `Exec`/`Cmd`/`Running`/`ProcessExec` (every child process), `Blocking` (worker-thread jobs), `Http`/`Request`/`Source` (the only `curl` argv), `Store`/`Codec`/`DiskStore`/`KeyedDiskStore`/`ConfigStore`/`MemoryStore`, `fsx` (logged file helpers, no silent `let _ =`), `Clock`, `Translator`, `Notifier`/`Notice`/`NotifySendNotifier`, `Feed`/`FeedSpec`/`Policy` (keyed data with refresh policy), `Daemon`/`DaemonSpec` (restartable line-streaming process with watchdog), `settings::derive` (typed settings from a section signal), `CacheFile`, `testing` (`ManualScheduler`, `FakeExec` deferred by default, `FakeClock`, `FakeTranslator`, `FakeNotifier`, `TestCtx`) |
| `src/platform/` | system integrations: `bridge` (`spawn_worker`, `spawn_thread`, `EventSink`), `dbus` (`BusKind`, test-only `TestBus`), `compositor/` (Hyprland, Niri, Sway, Mango), `upower`, `mpris`, `notifications` (`NotificationServer`, `post_local`, `LocalNotifier`), `bluetooth` (BlueZ and the pairing agent), `network` and `network_import/`, `tray`, `pipewire`, `toplevels`, `idle`, `screensaver`, `polkit`, `file_chooser` (XDG portal `FileChooser` client: `PickSpec`, `pick` with cancel on drop, `Outcome`, `probe` and `log_backend` for the startup check), `power_profiles` (power-profiles-daemon `ActiveProfile` and `Profiles`), `pam`, `pam_config`, `gamma` (also `kelvin_multipliers`), `brightness`, `desktop`, `desktop_service`, `app_matcher` |
| `src/lib.rs` | two `pub mod`s and a short `prelude`; no flat re-export list |

## For AI Agents

### Working In This Directory
- A new data service is a `FeedSpec` (key, policy, fetch, optional fallback) plus a `Codec` that keeps the existing on-disk format; a new long-running helper is a `DaemonSpec`; a new system integration goes in `platform/` with `spawn_worker` or `spawn_thread`. Never spawn a process or call `curl` or `notify-send` outside `kit/exec.rs`, `kit/http.rs`, `kit/notify.rs` (justified raw `Command::new`: the mmsg watcher in `platform/compositor/mod.rs`, the test-only `dbus-daemon` launcher, `polkit.rs` and `pam.rs` fork control).
- D-Bus recipe: define a `#[zbus::proxy]`, write `async fn run(bus, EventSink, Receiver)` that pushes whole snapshots, expose `Signal`s fed by the `spawn_worker` callback, test with `TestBus` plus a fake service built with `#[zbus::interface]`.
- zbus 6 differences: builders (`Builder::address`, `.path`, `.name`, `.serve_at`, `MatchRule::builder`) are infallible but `MatchRule...build()` returns `Result`; types are `zbus::Variant` / `OwnedVariant` (nested variant case is `Variant::Variant`); use `CacheProperties::No` when re-reading after `PropertiesChanged`; `async_channel::Receiver` is `!Unpin`, so `std::pin::pin!` merged streams.
- Never hold a `RefCell` borrow across a call that can re-enter (`if let Some(x) = cell.borrow().field` keeps the `Ref` alive through the body).
- Tests: use `kit::testing::TestCtx::new(root)` (deferred `FakeExec`: queue replies with `reply_ok`, then `release`/`release_next`; `ManualScheduler` for timers). `FakeExec::immediate()` hides ordering bugs, use it only for simple request/response tests.
- Hyprland dispatch: try the Lua form first (`hl.dsp.focus({ window = "address:0x.." })`), fall back to the legacy form. Verified live on Hyprland 0.56.2.
- Persisted and runtime files use the inherited paths and formats exactly (`.agents/archive/ROADMAP-legacy.md` §5b.0, ROADMAP rule 4); take every path from `kit::Dirs`.
- The M3R design is frozen in `.agents/archive/ROADMAP-legacy.md` §5b: read it before changing a service. Layering: this crate is engine code only; do not add Serpantinum policy to `platform`.

### Testing Requirements
`cargo test -p acanthus-services`. Needs `dbus-daemon` on PATH (provided by `shell.nix`).

## Dependencies
Internal: acanthus-core, acanthus-io. External: zbus 6, libloading (libpam), pipewire 0.10, wayland-client, wayland-protocols (staging), wayland-protocols-wlr, calloop-wayland-source, calloop, async-io, async-channel, futures-lite, serde, serde_json, rustix, chrono, image, regex.
