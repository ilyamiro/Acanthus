# Acanthus: how it is built, shipped and run (toolchain and tooling decisions)

> Decision record of 2026-10-27, written after checking Quickshell (its docs, its build file, the installed binary and the running process on this machine) and after a plugin-loading spike. Companion of `ROADMAP.md` and `ENGINE.md`. Status words: **decided** (user or measured), **proposed** (agent recommendation, change it with a reason), **verify** (assumption to prove with the listed item).

## 1. What "something like Quickshell" means

Facts gathered (sources: Quickshell BUILD.md, the introduction guide, `qs --help` of version 0.3.2 on this machine, `/proc` of the running instance):

| Quickshell fact | Consequence for Acanthus |
|---|---|
| One host binary (`quickshell`, alias `qs`) runs a **config**: a named folder `<xdg>/quickshell/<name>/shell.qml`, or any path (`-c`, `-p`, env `QS_CONFIG_PATH`). Options `-n` (no duplicate instance), `-d` (daemonize). | An `acanthus` host binary with the same shape: `<xdg>/acanthus/<name>/shell.serp`, `-c/-p/-n/-d` (E6.13). |
| The config **live-reloads on save**. | Markup hot reload (E6.8), built into the host. |
| Subcommand `ipc` talks to a running instance (`--id` with unique substring, `--pid`, same config selection), plus `list`, `kill`, `log`. | The same instance model: registry in `$XDG_RUNTIME_DIR/acanthus/`, `acanthus ipc call|show|prop`, `list`, `kill`, `log` (E6.13). Today's IPC socket and targets are the seed. |
| Module groups: core (`PanelWindow`, `FloatingWindow`, `Variants`, `Scope`, `Singleton`, `SystemClock`), `Io` (`Process`, `FileView`, `IpcHandler`, `Socket`), `Services.*` (Bluetooth, Mpris, Notifications, Pam, Pipewire, Polkit, SystemTray, UPower, Greetd), `Hyprland`, `I3`, `Wayland` (layer shell, toplevels, session lock, screencopy), `Widgets`, `DBusMenu`. | Same grouping already exists in `ENGINE.md` §3. |
| Every feature is a **CMake option, default on**; a missing system dependency must be switched off or the build fails (`-DSERVICE_PIPEWIRE=OFF`, `-DWAYLAND=OFF`, `-DX11=OFF`, `-DSCREENCOPY=OFF`, `-DHYPRLAND_GLOBAL_SHORTCUTS=OFF` ...). | Cargo **features** with the same granularity and default-on (§3.5). |
| **All QML modules are statically linked into Quickshell. It has no plugin system of its own.** A native plugin such as `~/Projects/pluginsquickshell` is a standard *Qt* QML plugin found through the import path (`qt_add_qml_module`, `.so`, `qmldir`); Quickshell must be rebuilt against each Qt release "or ABI mismatches can cause crashes". | Our native plugin story (same toolchain, rebuild per release) is the same bargain Quickshell users already accept. Unlike Quickshell, Acanthus makes it a first-class feature (§4). |
| Crash handler (cpptrace) with crash reports and a configurable report URL; symbols kept in release builds; jemalloc recommended because the QML engine fragments memory. | Crash handler with report files (E7.12); allocator chosen by measurement (§3.7). |
| `DISTRIBUTOR` string required for distro builds; QML install dir options. | `ACANTHUS_DISTRIBUTOR` build env shown in `acanthus --version` and crash reports. |
| Qt 6.6+, private Qt headers, clang builds much faster than gcc, Ninja only. | Our equivalent: one pinned rustc, mold + clang, Cargo only. |
| Disabling the unix socket "does not make running untrusted code safe". | Trust model: installing a config or plugin runs its code. **Decided: Acanthus uses the Quickshell trust model** (user decision 2026-10-27). |

### Why Quickshell is "heavy" (measured on this machine, 2026-10-27, the running Serpantinum QML shell)
`quickshell` process: **497 MB resident** (peak 781 MB; 304 MB anonymous heap, 193 MB file-backed shared libraries), **49 threads**, 2.4 GB virtual, plus a Python helper (22 MB), one `inotifywait` process per watched path and bash screenshot scripts. The weight comes from: the Qt and QML runtime and its JavaScript engine, the Qt Quick render thread plus worker threads, one scene graph per window, Mesa/Vulkan libraries mapped by each toolkit layer, and the glue processes the QML code spawns.

Acanthus avoids most of it by construction: no JS engine, no separate scripts or helper processes (native inotify, D-Bus and Wayland clients in the same process), one shared GPU context and glyph atlas for all surfaces, lazy popup surfaces, one UI thread plus small worker pools, 294 crates in the whole dependency tree and no async runtime (calloop + async-io). **None of this is measured for Acanthus yet**; the Mesa/Vulkan file-backed share is unavoidable and shared between processes. Measuring comes first (E7.13): do not claim a number before it exists.

## 2. Spike: can a separately built Rust plugin work with the host? (done, evidence)

Setup (scratch workspace, rustc 1.98.1, release, `-C prefer-dynamic`, `panic = "unwind"`): `acan_core` as a `dylib` holding a `thread_local!` registry, a generic `Signal<T>` and a `guarded()` helper using `catch_unwind`; `demo_plugin` as a `dylib` depending on `acan_core` and exporting `acanthus_build_id`, `acanthus_register(&Signal<i32>)` and a function that panics; `host` binary loading the plugin with `libloading`.

Result: host and plugin print the same build id; the plugin registered a component into the **same** thread-local registry the host reads (`["host-component","plugin-component"]`); a generic `Signal<i32>` created by the host was read and written by the plugin through a plain Rust-ABI reference (value 1 -> 42); a panic (index out of bounds) inside the plugin was caught by `guarded()` in the host, which carried on ("host still alive"); both `host` and the plugin link `libacan_core.so` and **one** `libstd-<hash>.so` dynamically (the hash is part of the file name, so a different rustc simply fails to load: a natural safeguard on top of the explicit build id).

Not proven yet (E7.11 must prove them): hard crashes, Wayland/wgpu objects crossing the boundary, thread-locals of dependencies like `wgpu` inside the shared library, size and startup cost of a real `libacanthus.so`, a plugin built from outside the workspace with a lockfile mismatch, behaviour with `lto = "thin"` in the host.

## 3. Decisions

### 3.1 Language and compiler: decided
Rust edition 2024. **One pinned stable rustc** (today nixpkgs 1.98.1) recorded in `rust-toolchain.toml` and in the Nix flake; bumps are deliberate, announced in the changelog, and every bump invalidates plugin binaries (build id contains the rustc version). No MSRV promise to third parties beyond "the pinned toolchain": same-toolchain plugins make a wider MSRV meaningless.

### 3.2 Build orchestration: proposed
Cargo workspace only. No CMake, make or just. Repository automation is a Rust `xtask` crate (`cargo xtask <task>`): `new-module`, `new-plugin`, `check-layers`, release packaging, golden-image regeneration. `scripts/check.sh` stays as the single quality gate and calls xtask where useful.

### 3.3 Linking and compile speed: decided (mold) / proposed (rest)
clang + mold (present). Dev profile: `debug = "line-tables-only"`, dependencies at `opt-level = 2` (present). Release: `lto = "thin"`, `codegen-units = 1`, **`panic = "unwind"` (required: plugin panic containment and the crash hook rely on unwinding)**, symbols kept (`debug = "line-tables-only"`, `strip = "debuginfo"`) so crash traces are readable, like Quickshell.

### 3.4 Two build modes: proposed (verify in E7.11)
- **static** (default, what Serpantinum ships today): one binary, thin LTO, fastest.
- **shared**: the engine crates are re-exported by one facade crate `acanthus` built as a `dylib`; host and plugins link it and the standard library dynamically (`-C prefer-dynamic`, as in the spike; Bevy's `dynamic_linking` is the known precedent). Only this mode loads plugins; the cost is slightly slower startup and no LTO across the plugin boundary, to be measured. Nix builds both.

### 3.5 Features instead of CMake flags: proposed
Every optional system dependency or protocol is a Cargo feature, default on, with the same granularity as Quickshell's flags, so a distro can drop what it does not have: `pipewire`, `pam`, `polkit`, `mpris`, `tray` (SNI + dbusmenu), `bluez`, `networkmanager`, `upower`, `notifications`, `hyprland`, `sway`, `niri`, `mango`, `session-lock`, `screencopy`, `toplevels`, `data-control`, `video` (FFmpeg), `crash-handler`. No X11 support is planned. Minimal system libraries when everything is off: wayland, xkbcommon, vulkan-loader, fontconfig/freetype.

### 3.6 Heavy dependencies stay few: decided in spirit, enforced by `cargo deny`
Already chosen and kept: wgpu (Vulkan only), sctk + wayland-protocols, calloop, cosmic-text + swash, zbus, pipewire, resvg, image (png/jpeg/webp/gif/bmp), rusqlite (bundled), regex. Rules: no async runtime (tokio), no GUI toolkit, no embedded scripting engine, no second text or image stack; any new dependency above a few hundred KB of code gets a line in the commit message and a check with `cargo tree -d` for duplicates.

### 3.7 Allocator: verify
glibc malloc today. Quickshell recommends jemalloc because its engine fragments memory; Acanthus may not need it. E7.13 measures RSS over a day of use with glibc, `mimalloc` and `tikv-jemallocator`; the winner becomes a feature. In shared mode the allocator is chosen by the host executable and used by plugins through the shared std, so there is exactly one.

### 3.8 Testing: proposed
`cargo nextest` as the runner (one process per test isolates thread-locals and speeds up the 1,860+ tests), `cargo test --doc` kept for doc tests. Headless GPU tests also run in CI on **lavapipe** (Mesa software Vulkan) so they do not skip. Golden PNGs as today. `cargo fuzz` for the parsers (`.serp`, calculator, lyrics, StyledText). Benchmarks with `divan` (frame build, layout, glyph upload). Coverage (`cargo llvm-cov`) on demand, not a gate.

### 3.9 Quality gates: proposed
`scripts/check.sh` = fmt, build, clippy `-D warnings`, nextest, comment rule, layering check, plus (when they exist) `cargo doc --no-deps` warning-free, `cargo deny check` (licences must be AGPL-3.0-or-later compatible, advisories, bans, duplicate versions), `cargo machete` (unused dependencies), `cargo semver-checks` once the API is declared stable (E7.6, E7.7).

### 3.10 Packaging: proposed
`flake.nix` with `crane` (dependency layer cached), a dev shell replacing `shell.nix`, a package for each build mode, an overlay, a NixOS and home-manager module; a `rustPlatform.buildRustPackage` derivation for upstream nixpkgs later. Arch (AUR) and Debian recipes after S9. `ACANTHUS_DISTRIBUTOR` set by each packager.

### 3.11 CI: proposed
GitHub Actions (the repository already lives on GitHub): Nix build with a binary cache, `scripts/check.sh` with lavapipe, example apps build, layering check, `cargo deny`. Release artifacts only after E7.

### 3.12 Diagnostics: proposed
`tracing` (present) with an optional `tracing-tracy` feature for frame profiling, an optional `dhat` feature for heap profiles, per-instance log files read by `acanthus log`, crash handler with a signal handler and backtrace writing a report next to the log (like Quickshell's crash reporter), and the existing panic hook (module and last action). Counter overlay for layout ms, primitives and fps (E4.20).

### 3.13 Markup tooling for E6: proposed, to confirm when E6.4 starts
`logos` lexer, hand-written recursive-descent parser with a Pratt parser for expressions, `ariadne` for located diagnostics, `tree-sitter` grammar for editors, `lsp-server` for the language server, `clap` (derive) for the `acanthus` and `acanthus-cli` command lines.

## 4. Plugins: what is decided and what is not

Decided with the user (2026-10-27):
- Native plugins built with the **same toolchain and the same Acanthus build** as the host, loaded as dynamic libraries. A pure stable-ABI or executable-only model is not required.
- Plugins can do **everything** the engine can: draw widgets, provide services and actions, register modules.
- Users install plugins written by other people, and write their own desktops and widgets with Acanthus.
- Trust model: **the Quickshell model**. Installing a config or a plugin runs its code with the user's privileges; no sandbox is promised.

Decided by the agent on the user's mandate (2026-10-27, "decide what is best"); change any of it with a reason:

1. **Panics are contained.** Every host-to-plugin entry is wrapped in `catch_unwind`: module start and stop, component build, action handlers, service callbacks, timers, IPC handlers, and every `Effect` or event handler that lives in a scope owned by a plugin. `Scope` carries an optional plugin id, set when the plugin registers; the reactive runtime and the event dispatcher check it and catch panics for plugin-owned scopes only (shell code keeps today's behaviour: the panic hook, then the process dies, because hiding bugs in the shell itself would be wrong). On a caught panic the plugin is **quarantined**: its scope is torn down (nodes, effects, timers, surfaces, IPC targets, registered actions), the shell shows a notification with the plugin name and panic message, and the rest of the desktop keeps running. A signal that was half-updated when the panic happened may hold a stale value; this is accepted. Plugins must be built with `panic = "unwind"`; the build id includes the panic strategy, so an `abort` build is refused at load.
2. **Blame is by what code was executing, not by "who was loaded".** A static atomic `CURRENT_PLUGIN` (async-signal-safe, one slot per thread that may run plugin code; engine helpers like `acanthus::spawn` tag worker threads) is set on every entry into plugin code and restored on exit. The crash handler (E7.12) and the panic hook write it into the report as `blamed_plugin`. Crashes outside plugin code blame nobody.
3. **Crash journal and quarantine rules.** The host keeps `crashes.json` in the state directory (timestamp, blamed plugin or none, report path). On the next start:
   - a plugin blamed in **two crashes within 24 hours** is **disabled** until the user runs `acanthus plugin enable <name>` (or edits the config); the user is notified at start;
   - a plugin blamed **once** starts normally, with a notification that the shell crashed while it was running;
   - **three crashes within five minutes with or without blame** start the host in **safe mode**: no plugins are loaded at all, the config's own `.serp` still is, and the notification says so; `acanthus plugin` and `acanthus log` still work; leaving safe mode is `acanthus reload --plugins` or the next normal start after ten quiet minutes.
4. **Freezes become blamed crashes.** A watchdog thread checks the event-loop heartbeat; if it stalls for **5 seconds while `CURRENT_PLUGIN` is set**, it writes the report (blaming that plugin) and aborts the process, so the supervisor restarts it and rule 3 applies. A stall outside plugin code only logs.
5. **A tiny supervisor ships in the host.** `acanthus run --supervise` (the default with `-d`) starts a minimal parent that holds no plugins, no GPU context and no window, spawns the real host as a child and restarts it on abnormal exit with backoff (1 s doubling to 30 s, reset after 60 s healthy), passing the crash journal. It works on every compositor with or without systemd; the systemd unit simply runs the supervisor. (Serpantinum's own supervision, `serpantinumd` in the QML version, is the same idea.)
6. **No special isolation tier.** Own-surface plugins do **not** run in a separate process by default: a second process means a second GPU context and a service proxy over IPC, which costs the memory this project is trying to save, and Quickshell has no such tier either. An author who wants isolation writes a standalone Acanthus app (the examples E7.1 to E7.4 show how) and talks to the shell over IPC. Revisit only if measurements (E7.13) or real plugin crashes show demand.
7. **Plugins are never unloaded.** In production "reload" means restart. With `--dev` the host watches plugin files, loads the new build under a unique temporary path, tears down the old plugin's scope and leaks the old library (warning after 20 reloads).
8. **Manifest and names.** Each plugin ships `plugin.toml`: `name`, `version`, `requires` (other plugins with version ranges, loaded in dependency order), `acanthus` (build id it was built for), `description`, and the exports it declares (components, services, actions, modules). The manifest drives listing, ordering and diagnostics, not security. Every registration is namespaced by the plugin name (`myplugin.Clock`); a collision with a built-in or another plugin is refused with a message. A panic or error inside `acanthus_register` means the plugin does not load.

Mechanics (from the spike and `libloading`, already a workspace dependency): the facade crate exports the registry and a `plugin!` macro that emits `acanthus_build_id` (acanthus version + commit + rustc version + enabled features) and `acanthus_register`; the host compares build ids before calling anything and refuses mismatches with a readable message; plugin authors build with `acanthus plugin build` against the installed SDK (source or Nix), and a plugin directory sits next to the config like a Qt import path (`<config>/plugins/<name>/{plugin.toml,lib<name>.so}`, plus `$XDG_DATA_DIRS/acanthus/plugins`). CLI: `acanthus plugin list|build|enable|disable|info`.
