# Engine: contract, parity matrix and milestones (E0 to E8)

> **Acanthus** is a standalone engine in its own repository (user decision 2026-10-28). This file is the live checklist; the master plan, rules, phase order and progress figure are in `ROADMAP.md`. History before the split: the items below marked done were completed in the frozen shell repository (`~/Projects/serp-rust`, crates `crates/engine/*`), which is the baseline E0 imports. Paths such as `crates/engine/acanthus-x` in older item text now read `crates/acanthus-x`; mentions of "the shell" refer to the frozen reference application.

## 1. Definition of "engine v1" (the finish line of this track)

A developer who has only the engine crates, `docs/engine/` and the examples can, in Rust and in `.serp` markup, with hot reload:

1. build a **bar / panel** on any edge of any output (layer surface, exclusive zone, input regions, autohide), per output, appearing and disappearing with monitors;
2. build **popups and menus** (lazy layer surfaces and `xdg_popup` with grab), with open/close animation, exclusive groups, Escape and click-outside handled by the framework;
3. build a **lock screen** on `ext-session-lock` that survives a crash of the shell;
4. build a **desktop widget** on the background layer and a **floating window app** (`xdg_toplevel`) with forms, lists and text editing;
5. use **native services** as signals (compositor state, MPRIS, notifications server, tray, UPower, PipeWire, NetworkManager, BlueZ, polkit, PAM, idle, portal file chooser, clipboard, screencopy) without writing a D-Bus line;
6. get **theming, i18n, settings with hot reload, IPC, supervision and a crash hook** from the framework by declaring a module, not by writing a `main()`;
7. use a **widget library** (buttons, sliders, dropdowns, dialogs, tabs ...) driven by theme tokens, and write new widgets against a stable **component contract**;
8. run it with a **CLI** (`acanthus check|fmt|preview|new`), an editor grammar and a documented, versioned language.

Anything outside this list is not an engine goal (no accessibility tree, no HDR/colour management, no particles, no web content) unless the user adds it.

---

## 2. Crate contracts (what each engine crate owns, must not do, and where it stands)

| Crate | Owns | Must not | State / leaks |
|---|---|---|---|
| `acanthus-core` | `Signal`, `Effect`, `Memo`, `Scope`, `Tracker`, `batch`, `untrack`, `Color` (lighten, darken, mix), `Rect`, `Vec2`, `ListSignal` | know about scenes, Wayland, files | clean. Open: height-ordered flush (E4.21) |
| `acanthus-anim` | Qt-exact easing, `Tween`, `Spring`, `Sequential`/`Parallel`/`Loop`, driver, `Animated<T>` (Behavior), `Keep` (ownership + `derive`/`ease`/`spring`/`alpha`) | draw | clean. Open: node-level `.animate` (E6.2) |
| `acanthus-color` | Material You colour science (HCT, quantiser, dynamic schemes), byte-exact with matugen 4.2.0 | know the shell's templates | clean; NOTICE for material-colors needed before release (E7.8) |
| `acanthus-image` | decode (raster, SVG), icon theme lookup, fill modes, async `ImageStore` | draw | clean. Open: SVG re-render at laid-out size, eviction, mipmaps (E4.17) |
| `acanthus-scene` | node arena, builder, layout (anchors, row, column, grid, flow, stack, constraints), text shaping, display list, hit testing, `MouseArea`, `Flickable`, `ListView`, `TextInput`, focus and keys, layers, shaders as nodes | talk to Wayland or the GPU | clean. Open: `Canvas` kind (E4.13), TextEdit/IME (E4.16), logical-unit scale (E3.10) |
| `acanthus-render` | wgpu context shared per format, per-surface renderer, SDF quads, glyph atlas (R8 mask + RGBA pages, LRU), images, layers, blur/shadow/mask/adjust, 30 `ShaderEffect`s | know the shell | clean except that the shader list holds Serpantinum-only effects (`eq_lightning`, `fingerprint_*`, `logo_water_wave` ...): E3 follow-up makes the effect registry app-extensible |
| `acanthus-wayland` | calloop + sctk `App`, layer surfaces, `per_output`, `per_output_lazy`, `App::popup` (xdg_popup), fractional scale + viewporter, pointer, keyboard, cursor shapes, idle inhibit | know module names | clean. Missing: toplevel, session lock, screencopy, data-control, DnD, text-input (E4) |
| `acanthus-io` | `Process`, `exec_detached`, `Watch` (inotify), `atomic_write`, timers, IPC socket (line JSON), `WeakHandle`, `Dirs` | name an app | **leak**: `dirs.rs` hard-codes `serpantinum` and `QS_*` (E3.3) |
| `acanthus-runtime` | `Paths`, `Config` hot reload, `I18n`, `Clock`, `Theme` + service, colour generation wiring, `Scaler`, font catalog, `Runtime` bundle | embed shell data | **leak**: embeds `config/serpantinum/settings.json`, assets root `share/serpantinum`, shell IPC targets (E3.3, E3.4) |
| `acanthus-services` | `kit` (Exec, Http, Store, Feed, Daemon, Notifier, settings helpers, Ctx) and `platform` (D-Bus bridge with supervision and shared executor, compositor backends, UPower, MPRIS, notification server, tray, PipeWire, NetworkManager, BlueZ, polkit, PAM, idle, toplevels, brightness, gamma, desktop entries, portal file chooser) | contain policy of one app | **mixed**: `feeds/` and `controllers/` are Serpantinum code (E3.5) |
| `acanthus-widgets` | theme-driven widgets: button/press feel, toggle, switch, slider, wheel, scrollbar, marquee, search field, dropdown, wallpaper backdrop; `timing` tokens | import the binary | depends on `acanthus-runtime::Theme` roles; target is tokens only (E3.4, D7) |
| `acanthus-app` (new) | module registry and `ModuleCx` (typed provide/require map), `Hub` with demand-started services, action registry mapped to IPC, popup manager (exclusive groups, Escape, click-outside, lazy surface), typed settings with selectors and schema export, supervision, crash hook | name a Serpantinum module | to be extracted from `crates/shell/serpantinum` (E3.6 to E3.9); built there already: `module.rs`, `actions.rs`, `dismiss.rs`, `hub.rs`, `crash.rs`, `ipc.rs` |
| `acanthus-markup` (new, E6) | parser with located errors, typed expressions, compiler to builder calls, hot reload | replace the Rust builder API | not started |
| `acanthus-cli` (new, E6.9) | `serp check`, `fmt`, `preview`, `new` | | not started |

The per-module contract that the shell follows and every new engine module must follow too (it is the reason 1,800 tests exist): `model.rs` (pure), `controller.rs` (state machine over a `Host` trait), `host.rs` (real services), `view.rs` (scene), `tests.rs`. `xtask new-module` scaffolds it (E3.13).

---

## 3. Parity matrix: Quickshell / Qt Quick capability -> engine home -> status

Status: **Done** (tested, wired), **Partial**, **Missing**. The item column points to the checklist below.

### 3.1 Quickshell core and Wayland

| Quickshell / Qt | Engine | Status | Item |
|---|---|---|---|
| `PanelWindow` (wlr layer shell), anchors, margins, exclusive zone, keyboard focus modes | `acanthus-wayland::LayerConfig` | Done | E1 |
| `Variants { model: Quickshell.screens }` | `App::per_output`, `per_output_lazy`, `App::outputs()` | Done | E1 |
| `LazyLoader` | `per_output_lazy` + `LazyGate`, `Scene::loader` | Done | E1, E3.12 |
| `PopupWindow` (xdg_popup, anchor rect, grab, nested) | `App::popup`, `PopupConfig` | Done | E4.1 |
| menus anchored to bars on `xdg_popup` with grab | | Missing | E4.4 |
| `FloatingWindow` (xdg_toplevel) | | Missing | E4.5 |
| `WlSessionLock` | | Missing | E4.6 |
| `ScreencopyView` | | Missing | E4.7 |
| `ToplevelManager` | `platform::toplevels` | Done | E2 |
| `IdleInhibitor`, `IdleMonitor` | `acanthus-wayland::idle_inhibit`, `platform::idle` | Done | E2 |
| `GlobalShortcut`, shortcuts inhibitor | | Missing | E4.9 |
| `HyprlandFocusGrab` | replaced by `xdg_popup` grab | Missing | E4.4 |
| `Quickshell.Hyprland / I3 / Niri` | `platform::compositor` (Hyprland, Niri, Sway, Mango) | Done | E2 |
| `Region` input masks | `Scene::input_region` | Done | E1 |
| `Scope`, `Singleton` | `acanthus_core::Scope`, `Hub` | Partial (Hub in the binary) | E3.6 |
| `SystemClock` | `acanthus-runtime::Clock` | Done | E2 |
| `Process`, `StdioCollector`, `SplitParser`, `execDetached` | `acanthus-io::Process`, `exec_detached`, `kit::Exec` | Done | E2 |
| `FileView`, `JsonAdapter` | `Watch`, `atomic_write`, `kit::Store` | Done | E2 |
| `Socket`, `SocketServer`, `IpcHandler` | `acanthus-io` IPC, targets via `Runtime` | Done (generic sockets Partial) | E2, E3.7 |
| `DesktopEntries` | `platform::desktop`, `AppMatcher` | Done | E2 |
| config reload on file change | `Config` hot reload | Done | E2 |
| hot reload of the UI code | markup hot reload | Missing | E6.8 |
| `qs` CLI (`qs ipc call`, `qs -c`) | `acanthus` CLI | Missing | E6.9 |

### 3.2 Native services (`Quickshell.Services.*`, `Bluetooth`, `Networking`)

| Quickshell | Engine | Status |
|---|---|---|
| Mpris | `platform::mpris` | Done |
| Notifications (server) | `platform::notifications` | Done |
| Pipewire | `platform::pipewire` | Done |
| SystemTray (SNI + dbusmenu) | `platform::tray` | Done |
| UPower, power profiles | `platform::upower`, `power_profiles` | Done |
| Polkit agent | `platform::polkit` | Done |
| Pam | `platform::pam` | Done |
| Bluetooth | `platform::bluetooth` (with pairing agent) | Done |
| Networking | `platform::network` | Done |
| Greetd | | not needed by the shell; add only if an app needs it |
| portal FileChooser client | `platform::file_chooser` | Done |
| clipboard (data-control) | | Missing, E4.8 (shell uses `cliphist` meanwhile) |
| screen capture | | Missing, E4.7 |

### 3.3 Qt Quick

| Qt Quick | Engine | Status | Item |
|---|---|---|---|
| Item, Rectangle (radius, per-corner radii, border), Text, Image, `ClippingRectangle` | `acanthus-scene` | Done | E1 |
| Layouts: Row, Column, Grid, Flow, Stack, anchors, constraints | `acanthus-scene` | Done | E1 |
| Repeater, ListModel keyed diff, Loader | `ListSignal`, `Scene::repeater/loader` | Done | E1 |
| Flickable, ListView (virtualised) | `Flickable`, `Scene::list_view` | Done | E1 |
| ListView highlight, add/remove/displaced transitions, GridView, ScrollView | | Missing | E4.15 |
| TextInput (selection, clipboard shortcuts, password, validator) | `TextInput` | Done | E1 |
| TextEdit (multi-line), undo, IME | | Missing | E4.16 |
| MouseArea, HoverHandler, cursor shapes, focus, keys | `MouseArea`, focus, `KeyEvent` | Done | E1 |
| DragHandler, Drag, DropArea | | Missing | E4.10 |
| Transforms, z, opacity, group opacity (`layer.enabled`) | `Node::{scale, rotation, z, layer}` | Done | E1 |
| `layer.effect`: blur, shadow, mask, colour adjust | `LayerEffect` | Done | E1 |
| `ShaderEffect` (sourceless, textured, data buffer) | `ShaderEffect` (30) | Done | E1 |
| Canvas, Shapes (paths) | | Missing | E4.13 |
| Video, MediaPlayer | | Missing | E4.14 |
| StyledText subset | `Node::styled` | Partial (no links, lists, headings) | E4.15 |
| Text: justify, capitalization, SDF scaled text | | Missing | E4.12, E4.15 |
| Animations: Number, Color, Spring, Sequential, Parallel, Pause, Loop, Behavior | `acanthus-anim` | Done | E1 |
| `Behavior on x` as a node property | `.animate(prop, motion)` | Missing | E6.2 |
| Qt Quick Controls | `acanthus-widgets` | Partial | E5 |
| fractional scale, viewporter | `acanthus-wayland::scale` | Done | E4.2 |
| damage regions, opaque region | | Missing (wgpu 30 limits, decision) | E4.19 |

---

## 4. Milestones and checklists

Item credit is 1 unless a credit is written. Percentages feed `ROADMAP.md` §3.

### E0 Standalone baseline (weight 4, 5 items, 1 done = 20 %)
- [ ] E0.1 Import the engine from the frozen repository as one `chore:` commit naming the source commit: `crates/engine/*` to `crates/`, `examples/`, `xtask/`, `docs/engine/` to `docs/`, `CHANGELOG.md`, `version.txt`, `deny.toml`, `shell.nix`, `.github/workflows/ci.yml`, a new root `Cargo.toml` (members listed explicitly, `[workspace.package].repository` of Acanthus, shared dependency versions, `Cargo.lock` regenerated and compared with the old one for drift). `git-filter-repo` is not installed; history stays in the frozen repository
- [ ] E0.2 Fixtures: the four engine reads of `../../../assets` (`acanthus-runtime/src/paths.rs`, `tests/matugen_oracle.rs`, `acanthus-render/src/atlas.rs`, `acanthus-scene/src/scene.rs`) read `assets/` of this repository (test fonts with licence notes, matugen fixtures) through one helper per crate. Acceptance: `grep -rn '\.\./\.\./\.\./' crates` empty
- [ ] E0.3 `scripts/check.sh` and `scripts/check-layers.sh` for a single-layer repo: no manifest path outside the repository, no `serpantinum`/`QS_` outside the allowed documents, `layers.allow` deleted; the `xtask new-module` templates pass the same check
- [ ] E0.4 Fresh-clone proof: `git clone` into a scratch directory, `HOME` pointing at an empty directory, `scripts/check.sh` green; the test count equals the engine share of the frozen repository (record both numbers in HANDOFF); CI workflow runs the same
- [x] E0.5 (2026-10-28) Tooling for separate development: `/acanthus` skill, `.claude/hooks/next-slice.sh` (marker file content names the skill; empty marker uses the repo default), `scripts/acanthus-tmux.sh`, `AGENTS.md`, `ROADMAP.md`, `HANDOFF.md`

### E1 Rendering and scene core (weight 13, 100 %)
Finished: old M0 and M1.1 to M1.11 (per-scene invalidation, text features, layout features, images and icons, transforms and rounded clipping, layers, blur/shadow/mask/adjust, 30 shader effects incl. textured and data-buffer effects, repeater/loader, flickable/list view, keyboard and text input, popups, multi-output, fractional scale). Detailed history: `archive/ROADMAP-legacy.md` §14 "M1 sub-milestones" and `archive/HANDOFF-history.md`. Remaining quality gaps are items in E4, so E1 stays 100 %.

### E2 Runtime and native services (weight 9, 98.3 %)
Finished: old M2 (95 %: the missing 5 % was "IPC targets are added per module", which is permanent, not a gap) and M3 (100 %), including the M3R restructure (`kit`, `Feed`, `Daemon`, typed settings, `Hub`, supervision, shared D-Bus executor and connections, notification and screensaver server retry, panic hook). The shell leaks inside these crates are E3 items, not E2 gaps.

### E3 Separation and app framework (weight 8, 16 items, 13.6 credits = 85.0 %)
Evidence and reasoning: `ARCHITECTURE-REVIEW.md`. Every item is a `refactor:`/`perf:`/`feat:` commit with the full definition of done green and no visual change.

- [x] E3.2 Physical split: `git mv` the crates into `crates/engine/` and `crates/shell/` (engine: core, anim, color, image, scene, render, wayland, io, runtime, services, widgets; shell: serpantinum-audio, serpantinum), update `Cargo.toml` members and workspace paths, `scripts/check.sh`, `shell.nix`, `include_str!` depths, every `AGENTS.md`. Test count identical. Done 2026-10-27.
- [x] E3.14 Rename to Acanthus: engine crates `serp-*` -> `acanthus-*` (directories, package names, `use` paths, `AGENTS.md`, docs), `serp-audio` -> `serpantinum-audio` (shell), CLI name `acanthus`; mechanical `move` commit right after E3.2, test count identical; logo/wordmark from `~/Projects/Acanthus/` go into the docs later (E7.5).
- [ ] E3.15 Build foundation (**credit 0.5, 2026-10-27**: `rust-toolchain.toml` 1.98.1, release `panic = "unwind"` + line tables + `strip = "debuginfo"`, `cargo nextest` and `deny.toml` + `cargo deny check licenses bans sources` in `check.sh`, `acanthus-services` features `pipewire`, `pam`, `polkit` (default on, `--no-default-features` compiles, checked in `check.sh`); open: `flake.nix` with crane, remaining features such as `mpris`, `tray`, `bluez`, `networkmanager`, compositor backends, `cargo xtask`) (`TOOLCHAIN.md` §3): `rust-toolchain.toml` pinned to the Nix rustc, `flake.nix` with crane (dev shell, package, overlay), release profile with `panic = "unwind"` and kept symbols, Cargo features for every optional service and protocol (default on), `cargo xtask` crate, `cargo nextest` as runner, `deny.toml` + `cargo deny check` in `check.sh`. Mostly independent of the other E3 items; do it right after E3.14.
- [x] E3.1 `scripts/check-layers.sh` (rules in ROADMAP §3.1) wired into `scripts/check.sh`, with `scripts/layers.allow` listing today's violations; the file only shrinks.
- [x] E3.3 App identity: `AppId { name, env_prefix }` in `acanthus-io`, `Dirs::{from_env, resolve, from_vars, under}` and `Paths::{from_env, resolve}` take it; assets variables are `<NAME>_ASSETS` / `<NAME>_DIR`; engine tests use the test app `demo` / `DM`. Serpantinum passes `serpantinum` / `SERPANTINUM` (the `QS` prefix is gone, see E3.16). Done 2026-10-27; `services::kit::app_id()` is a temporary home for the bundled feeds and leaves with E3.5.
- [x] E3.16 No Quickshell naming (user decision 2026-10-27): no `QS_*` variable, no `quickshell`/`qs-master` identifier or wording in code, settings, docs or UI text (`focus.rs` shell-window detection, `pam.rs` test names, `Dirs` rustdoc, `SHELL.md` tables excepted where they cite the QML source). The `QS_*` override variables are replaced by `SERPANTINUM_*` (done in E3.3, the QML shell's scripts are not copied). **Data stays where it is**: the directories and file formats under `~/.config|.cache|.local/state/serpantinum` and `$XDG_RUNTIME_DIR/serpantinum` (focus time, launch stats, cached images, lyrics, ...) are kept so the Rust shell migrates from the QML shell without losing data; only the names of variables, identifiers and strings change. Acceptance: `scripts/check-layers.sh` also rejects `QS_` and `quickshell` (case-insensitive) in `crates/`, grep is clean. Done 2026-10-27: `focus::ShellMask` (app-supplied label and needles) replaces the hard-coded shell masking, `qs_colors.json`/`qs_matugen_colors.json` became `colors.json`/`matugen_colors.json`, launcher widget prefix `widget-`, language strings say Serpantinum, the check also covers `assets/` and `config/`.
- [x] E3.5 Services split (done 2026-10-27: `feeds/` and `controllers/` moved to `crates/shell/serpantinum-services` unchanged, `kit::app_id()` removed, `DisplayDevice::new` public, OpenVPN import needs `ImportOptions::output_dir` for inline certificates instead of a hard-coded data dir; every controller stayed in the shell, none was promoted): `acanthus-services` keeps `kit` + `platform`; `feeds/` and `controllers/` move to `crates/shell/serpantinum-services` (weather, location, updater, lyrics, art, equalizer, audio_route, lock_auth, sysnotif, power, window_tracker, ...). Anything generic found inside a controller (recording watcher, kb locks, sounds, focus tracker, window tracker, power command chains) is judged by ROADMAP §3.2 and may stay in the engine behind a generic API.
- [x] E3.4 (done 2026-10-27: shader effects the shell owns live in `serpantinum-effects` and register through `acanthus_scene::register_app_effects`; earlier: `acanthus-widgets` no longer depends on `acanthus-runtime` (`SearchTokens` replaces `Theme`, the shell maps its theme to tokens), IPC targets `theme`, `fonts`, `matugen`, `config` are app-neutral; assets root fallbacks are a `Paths::from_env_with` argument (matugen templates and theme presets live under it); default settings are a `Runtime::start` argument and `Config::load` has none; platform agent names, PAM service, virtual sink and notification server name are parameters; `layers.allow` is empty. ) Runtime and theme de-shelling: default settings, assets root, matugen templates, theme presets and IPC target names are supplied by the app; `Theme` exposes tokens (colour roles per D7, motion, spacing, radii, fonts) and `acanthus-widgets` depends on tokens only; effect registry accepts app-registered effects.
- [ ] E3.6 `acanthus-app`: module registry, `ModuleCx` with typed `provide`/`require`, `Hub` with demand-started services that stop when the last handle drops, bar-independent. **Credit 1.0 (2026-10-28)**: new crate `acanthus-app` holds `Module<Cx>`, `start_all` (stage callback for crash reports), `Provided` (typed provide/require map) and `Slot<T>` (shared service started on first request, `Slot::demand` stops it with the last handle); the shell `ModuleCx` wraps `Provided` and `Hub` is built from `Slot`s (kept strong, behaviour unchanged). Open: the Hub service list itself is Serpantinum-specific (stays in the shell), `NameOwnerChanged` restart, switching optional services to `Slot::demand` where safe.
- [ ] E3.7 Action registry: named typed actions, verbs and aliases, exclusive groups, `trigger`, mapped onto IPC `handleCommand` names; keybindings and markup use the same path. **Credit 0.8 (2026-10-27)**: `acanthus_app::{Actions, Verb}` extracted (names and aliases, exclusive groups, configurable dismiss group, dispatch observer for the crash hook, `Actions::serve(ipc, target, method)`), 11 tests moved; the shell keeps `STACK`, the popup list (`ipc.rs`) and `actions::new()`. **Credit 0.9 (2026-10-28)**: `Actions::register_typed::<A: FromStr>` (empty argument is `None`, unparsable is logged and skipped; 1 test). Open: keybinding source (E4.9), markup binding.
- [ ] E3.8 Popup manager: `Popup` trait, exclusive groups, Escape, click-outside (`dismiss.rs`), lazy surface; the shell registers its popups only. **Credit 0.95 (2026-10-27)**: `acanthus_app::dismiss` (`Spec`, `free_area`, `regions`, `bind_input_region`, `attach`) holds the click-outside layer and input region maths, parameterised by a `reserved` strip instead of the bar; `LazyGate` already lives in `acanthus-wayland`; the shell keeps `bar_reserved` and `STACK`. `acanthus_app::Popup` (names, group, is_open, open, close, toggle) registers through `Actions::add`; `Actions::escape` and `open_popups` close/list open popups; the shell's seven popups register through it in `ipc.rs`. `acanthus_app::popup::closing_on_escape(close, keys)` wraps a popup's key handler so Escape closes it centrally; calendar, clipboard, music and volume use it (the dock picker's Escape leaves edit mode, not a popup). E3.8 complete.
- [ ] E3.9 Typed settings: serde-derived sections with per-field selectors (`Memo`), schema export for the settings app, validation errors; Serpantinum's sections move to the shell. **Credit 0.75 (2026-10-27)**: `acanthus_runtime::Section<T>` (`Config::section`) parses a path with serde, keeps the last good value, reports `errors`, and `select` gives per-field memos (3 tests). **Credit 0.9 (2026-10-28)**: `Section::schema()` exports path, nested fields, JSON types and defaults from `T::default()` (1 test). **Part 3 (2026-10-28)**: the eleven bar face settings in `bar/settings.rs` read through `Config::section` with a `Deserialize` shim (`Face<S: FromBar>`, tolerant parsers kept). Open: workspaces (three paths) and `environment.rs` stay on hand parsers; the reason: their parsers accept legacy flat keys and truthy coercions that plain serde would reject, so each needs a `Deserialize` shim first.
- [ ] E3.10 Logical units (part 1 2026-10-28: `acanthus_core::scaled` is the one rounding rule, 60 shell copies call it; scene-applied scale open): views write logical units and the scene applies the UI scale with the `Scaler.s` rounding; module by module with pixel-compare tests (about 1,600 scale multiplications today).
- [x] E3.11 Supervision: `spawn_supervised` with backoff, shared D-Bus executor and connections, supervised notification server and screensaver retry, panic hook with module and last action (R2.1, R2.2 partly, R2.4, R2.5). Follow-ups that belong to other items: restart on `NameOwnerChanged` (E3.6), lock as its own binary (S6).
- [x] E3.12 GPU and surface economy: shared `GpuContext`, R8 mask atlas with LRU pages, lazy per-output surfaces, mold linker (R1.0, R1.2, R1.3). Open measurement: VRAM and startup before/after (needs a launch).
- [x] E3.13 (module contract in `acanthus-app/AGENTS.md`; `cargo xtask new-module <name>` scaffold, 4 tests, generated module compiled and passed in the shell crate 2026-10-28) Module contract documented (`docs/engine/` once E7.5 starts, `crates/engine/acanthus-app/AGENTS.md` before) and `xtask new-module <name>` scaffolds model/controller/host/view/tests with a passing test.

Mapping from the old M5R list: R1.0 -> E3.12, R1.1 (done, in E2/E1 crates), R1.2 -> E3.12, R1.3 -> E3.12 (R1.3c live check stays in `SHELL.md`), R1.3b -> E4.4, R1.4 (done), R1.4b/R1.4c -> E6.3/E6.2, R1.5 -> E5, R1.6 -> E3.6, R1.7 -> E3.7/E3.8, R1.8 -> E3.6, R2.1 and R2.2 and R2.4 and R2.5 -> E3.11, R2.3 -> S6, R3.1 -> E3.9, R3.2 -> E4.20, R3.3 -> E3.10, R3.4 -> E6.1, R3.4b -> E3 follow-up (per-module crates, build time), R3.5 `BarGeometry` -> `SHELL.md` S4 follow-up, R3.6 -> E4.13, R3.7 -> E4.21, R3.8 -> E4.8.

### E4 Platform surfaces and render completeness (weight 14, 21 items, 3 done = 14.3 %)
- [x] E4.1 Popup surface API (`App::popup`, anchor rect, gravity, constraint adjustment, grab, nested)
- [x] E4.2 Fractional scale + viewporter with integer fallback
- [x] E4.3 Animated images (`animated(true)`)
- [ ] E4.4 Menus on `xdg_popup` with grab (tray menu, dock menu): click-outside and stacking from the compositor; popup surfaces sized to their bounds plus the animation margin where the visual allows (R1.3b)
- [ ] E4.5 Toplevel windows: `xdg_toplevel` (`FloatingWindow`): title, app id, min/max size, close request, interactive move/resize, decoration policy; keyboard focus and input like layer surfaces
- [ ] E4.6 `ext-session-lock-v1` client: lock surface per output, `locked`/`finished` signals, unlock, surface recreation on output change; behaviour when the client dies (compositor keeps the session locked) documented and tested with a fake
- [ ] E4.7 Screen capture: `ext-image-copy-capture-v1` / `wlr-screencopy`, a `ScreencopyView` node (live and one-shot), buffer import path chosen (shm first)
- [ ] E4.8 Clipboard: `ext-data-control-v1` / `zwlr_data_control_v1` read, write, watch with MIME types; `TextInput` uses the system clipboard; the shell drops `cliphist` + `wl-paste --watch` (R3.8)
- [ ] E4.9 Global shortcuts: compositor-agnostic registration (Hyprland global shortcuts, Niri/Sway bindings through their IPC, documented fallbacks) as actions (E3.7)
- [ ] E4.10 Drag and drop: `wl_data_device` source and target, in-app `Drag`/`DropArea`/`DragHandler` semantics
- [ ] E4.11 Exact nested rounded and rotated clips (stencil or mask pass or a clip stack); today only the innermost clip is exact
- [ ] E4.12 Scaled and rotated text quality (SDF glyph atlas or re-rasterisation at the scaled size)
- [ ] E4.13 `Canvas` node kind: custom paint from a closure recording primitives; path/stroke/fill primitives for graphs and the Draw tool (R3.6)
- [ ] E4.14 Video: minimal FFmpeg pipeline (decode on a worker, upload, optional dmabuf import via wgpu-hal), `Video` node, loop and mute, hardware decode where available
- [ ] E4.15 Text and list completeness: justified text, `font.capitalization`, bidi/RTL verification suite, StyledText links/lists/headings; `ListView` highlight, add/remove/displaced transitions, `GridView`, `ScrollView`, `flickBoost`
- [ ] E4.16 Text editing: multi-line `TextEdit`, undo/redo, `text-input-v3` IME and compose, selection by pointer drag across lines
- [ ] E4.17 Images: SVG re-render at laid-out size, store eviction for unreferenced images, mipmaps for large downscales, `Image.Pad`
- [ ] E4.18 Device-lost and surface-lost recovery (rebuild context and surfaces without restarting the app)
- [ ] E4.19 Damage and opaque regions: decide and implement a present path (wgpu 30 does not expose `damage_buffer`; options: raw `VK_KHR_incremental_present` hook, or accept full commits and document the measured cost) (R1.3c)
- [ ] E4.20 Incremental layout: dirty flags with relayout from the nearest boundary, subtree display-list reuse, static props, frame-callback animation clock, profiling overlay (layout ms, primitives, fps); data first (R3.2)
- [ ] E4.21 Glitch-free reactive flush: height-ordered `Memo` so diamond dependencies do not run effects with half-updated inputs (R3.7)

### E5 Widget library (weight 12, 20 items, 9 done = 45 %)
All widgets take theme tokens, expose a `Spec` struct, follow the component contract (E6.1) once it exists, and have view tests. Shell-specific styling stays in the shell.
- [x] E5.1 Button and press feel (`Press`, `Feel`, icon button, focus ring, flash, click sound hook)
- [x] E5.2 Toggle
- [x] E5.3 Switch
- [x] E5.4 Slider (pointer, wheel accumulator, arrow/Home/End keys)
- [x] E5.5 Scroll bar and scroll handle
- [x] E5.6 Marquee text
- [x] E5.7 Search field (text input + clear button)
- [x] E5.8 Dropdown (staggered menu, scroll bar, click-outside, double-click search with fuzzy filter)
- [x] E5.9 Wallpaper backdrop (blurred snapshot under translucent surfaces; snapshot source supplied by the app)
- [ ] E5.10 Popup frame (card with radius, base fill, border, shadow, backdrop slot, enter/exit morph) replacing the ten hand-built popup bodies
- [ ] E5.11 Number selector
- [ ] E5.12 Time selector
- [ ] E5.13 Tabs and segmented control (extract the media popup switch)
- [ ] E5.14 Tooltip
- [ ] E5.15 Dialog and modal with scrim
- [ ] E5.16 Menu and context menu (uses E4.4)
- [ ] E5.17 Progress bar and spinner
- [ ] E5.18 Checkbox and radio group
- [ ] E5.19 Text area (needs E4.16)
- [ ] E5.20 Chart and sparkline (needs E4.13), wavy seek bar and fluid wave extracted from the shell

### E6 Authoring and host (weight 20, 13 items, 0 %)
Rules fixed by earlier decisions: the Rust builder API stays the stable core and markup compiles to exactly the same calls; no JavaScript, expressions are a small typed language compiled to closures inside an `Effect`; syntax close to QML so porting is mechanical; extension `.serp` (to be confirmed by the user when E6.4 starts); designed for public use (versioned syntax, stable native components, whitelisted functions). **Do not design the language before the P3/P4 prerequisites and the hello examples exist** (ROADMAP §4, D3): a language designed too early fits only what exists.
- [ ] E6.1 `Component`/`Props` contract: typed named defaultable props, each static or bound, a name registry, `build(cx, props) -> NodeId`; new widgets follow it from day one (R3.4)
- [ ] E6.2 Node-level `.animate(prop, motion)` (QML `Behavior on x`) replacing `keep.derive` + `keep.ease` (R1.4c)
- [ ] E6.3 Design tokens in the theme: motion (FAST/BASE/SLOW and the common literals), spacing, radii; views stop carrying literals (R1.4b)
- [ ] E6.4 `.serp` syntax specification v0 written from the patterns of the shell and the examples; versioned
- [ ] E6.5 Parser with `file:line:col` errors
- [ ] E6.6 Expression language: literals, property paths, arithmetic, comparison, ternary, calls into a whitelist (`t("key")`, `mix`, `fmt_time` ...)
- [ ] E6.7 Compiler to builder calls; native components and services registered as globals; `Behavior`/`Spring` mapping to `Animated`
- [ ] E6.8 Hot reload: watch files, rebuild the affected subtree, keep service-owned signal state, dev error overlay instead of a crash
- [ ] E6.9 `acanthus-cli`: `check`, `fmt`, `preview`, `new`
- [ ] E6.10 Editor support: tree-sitter grammar first, language server later
- [ ] E6.11 Proof: one real shell module (a popup such as the calendar) rewritten in markup with identical view tests
- [ ] E6.12 Language stability policy: versioning, deprecation rules, compatibility tests
- [ ] E6.13 `acanthus` runtime host (the `quickshell` equivalent, `TOOLCHAIN.md` §1): config discovery `<xdg>/acanthus/<name>/shell.serp`, `-c/-p/-n/-d`, live reload, instance registry in `$XDG_RUNTIME_DIR/acanthus/`, `ipc call|show|prop` with `--id`/`--pid`, `list`, `kill`, `log`, `reload`, `--supervise` parent, built-in modules behind Cargo features, plugin directory `<config>/plugins/` (plugin rules in `TOOLCHAIN.md` §4)

### E7 Proof and release (weight 10, 14 items, 6 done)
- [x] E7.1 Example `hello-bar` (2026-10-28, `examples/hello-bar`): a bar with a clock and one service signal (UPower battery), engine crates only, model/view split, 3 tests; no live run yet (a second GUI binary needs the user's say-so)
- [x] E7.2 Example popup app (2026-10-28, `examples/hello-popup`): a greeting card with lazy surface, `Popup` + `Actions` served over IPC, Escape through `closing_on_escape`, 4 tests; no live run yet
- [x] E7.3 Example desktop widget on the background layer with services (2026-10-28, `examples/hello-widget`: clock plus MPRIS track, `Layer::Background`, 2 tests; no live run yet)
- [ ] E7.4 Example floating-window app with forms, list, text editing (needs E4.5, E5)
- [ ] E7.5 Documentation (**no credit yet**; 2026-10-28 `acanthus-core`, `acanthus-anim`, `acanthus-image` and `acanthus-io` are fully documented with `#![deny(missing_docs)]`; missing-docs counts of the others measured with `RUSTFLAGS='-W missing_docs'`: color 541, app 1098, rest not counted yet): rustdoc on every public item of every engine crate (`#![deny(missing_docs)]` per crate as each is finished, `cargo doc --no-deps` warning-free in `check.sh`), plus guides in `docs/engine/`: architecture, reactive core, scene and layout, animation, rendering, surfaces, services, writing a module, writing a widget, markup. Backlog is tracked per crate (public items documented / total). New or changed public items are documented from now on (rule 1)
- [ ] E7.6 Public API list per crate (what is stable), `cargo public-api`-style check in CI
- [x] E7.7 (2026-10-28: `docs/engine/versioning.md`, root `CHANGELOG.md`) Semver and changelog policy; deprecation rules
- [x] E7.8 (2026-10-28: `acanthus-color` fixed from GPL to AGPL expression, `NOTICE` added, `check-layers.sh` fails on any other manifest licence; asset attribution list still to collect) Licence applied (AGPL-3.0-or-later, D4): workspace `Cargo.toml` and every crate say `AGPL-3.0-or-later` (`acanthus-color` is `AGPL-3.0-or-later AND (MIT OR Apache-2.0)`), `LICENSE.md` matches; `NOTICE` crediting material-colors and matugen
- [ ] E7.9 Benchmark and profile suite with the budgets of §8
- [x] E7.10 (2026-10-28: `.github/workflows/ci.yml` runs `scripts/check.sh` in the Nix shell on push and pull request, with dbus and Mesa lavapipe installed; first remote run not seen yet) CI: `scripts/check.sh` (headless GPU tests skip without an adapter), layering check, examples build
- [ ] E7.11 Native plugins (D2, `TOOLCHAIN.md` §2 and §4): shared build mode with the `acanthus` facade dylib, `plugin!` macro and build id (version + commit + rustc + features + panic strategy), `plugin.toml` manifest with namespacing and `requires` ordering, loader (`libloading`, never unloads, `--dev` reload by leaking), `acanthus plugin list|build|enable|disable|info`, example plugin that draws a widget and provides a service; first prove the open points of the spike (hard crashes, wgpu objects across the boundary, size and startup of the real shared library)
- [ ] E7.12 Crash handler and reports: signal handler with backtrace, report file next to the log including `blamed_plugin`, `crashes.json` journal, `ACANTHUS_DISTRIBUTOR` in reports and `--version`, symbols kept in release
- [ ] E7.14 Plugin containment and supervision (`TOOLCHAIN.md` §4 rules 1 to 5): plugin id on `Scope`, `catch_unwind` at every host-to-plugin entry and for plugin-owned effects and handlers, quarantine that tears the scope down with a notification, `CURRENT_PLUGIN` blame slots, two-strikes disable and safe mode from the journal, event-loop watchdog (5 s while in plugin code), `acanthus run --supervise` parent with backoff; tests with fake plugins that panic, loop forever and abort
- [ ] E7.13 Baseline measurements and budgets: RSS, threads, startup and VRAM of hello-bar and of Serpantinum over a day of use, compared with the measured QML shell (497 MB resident, 49 threads, 2026-10-27); allocator comparison (glibc, mimalloc, jemalloc); static versus shared build cost; budgets written into §8 afterwards

### E8 Consumer readiness (weight 10, 12 items, 0 %)
Each probe is an example application (markup where E6 allows, Rust before) built from the engine crates alone, with tests that run without a compositor, and proves one category of ROADMAP §5. Probes are small but structurally faithful: same surface type, same service wiring, same interaction pattern as the Serpantinum module they stand for.
- [ ] E8.1 Bar probe: top and side bars, several faces fed by services, per-output, autohide, exclusive zone, input regions
- [ ] E8.2 Popup probes: media, calendar, volume, network list, clipboard list, launcher; exclusive groups, Escape, click-outside, morph animation, popup frame
- [ ] E8.3 Dock probe: hover cascade, running indicators, edge trigger, context menu on `xdg_popup`
- [ ] E8.4 Lock probe: own binary on `ext-session-lock`, PAM, fingerprint signal, backdrop from screencopy, survives a crash of the host
- [ ] E8.5 Wallpaper probe: image and video per output, shader transitions
- [ ] E8.6 Desktop widget probe: background layer, drag and resize redactor, custom drawn graph
- [ ] E8.7 Settings app probe: floating window, schema-driven forms with search, tabs, dialogs, number and time selectors, portal file choosing, at least 20 pages generated from markup
- [ ] E8.8 Notifications, OSD and tray probe: server, popup stack, DND, dbusmenu
- [ ] E8.9 Visualiser probe: audio spectrum and curve editor drawn by canvas and shader
- [ ] E8.10 Migration guide `docs/migration-from-quickshell.md`: QML/Quickshell to `.serp`/Acanthus mapping table, data-directory compatibility rules, validated by mechanically porting two real QML files and comparing view tests
- [ ] E8.11 Performance acceptance: a composite of the probes meets the budgets of §8 (RSS, threads, idle CPU, startup) against the measured QML shell
- [ ] E8.12 Release `v1.0.0`: public API list frozen (E7.6), docs book complete (E7.5), changelog, tag, decision D9 asked, and the statement in `HANDOFF.md` "engine v1 ready, start the Serpantinum copy (ROADMAP §8)"

---

## 5. Extension points of the app framework (the seam between engine and applications)

| Point | Contract | Where |
|---|---|---|
| Module | `trait Module { fn start(cx: &ModuleCx) -> Box<dyn Running>; }` declares surfaces, IPC methods, actions, settings section | `acanthus-app` |
| Services | `Hub::get::<T>()` demand-starts a service and stops it with its last handle | `acanthus-app` + `acanthus-services` |
| Actions | `Actions::register(name, verbs, group, handler)`, `trigger(name, args)`; IPC, clicks, keybindings and markup resolve to it | `acanthus-app` |
| Popups | `Popup { open, close, toggle, is_open, exclusive_group }` managed centrally | `acanthus-app` |
| Settings | typed section structs, per-field `Memo` selectors, schema export | `acanthus-app` + `acanthus-runtime` |
| Theme | tokens + providers (preset files, generated scheme) | `acanthus-runtime` |
| Components | `Component { type Props; fn build(cx, props) -> NodeId }` + name registry | `acanthus-widgets` + `acanthus-markup` |
| Effects | app-registered `ShaderEffect`s through a registry | `acanthus-render` |
| Plugins (D2) | `plugin!` macro registers components, services, actions and modules into the registries of the shared engine; build id checked before use | `acanthus-app` + facade crate `acanthus` |
| Feeds / daemons | `FeedSpec`, `Daemon` declared by the app, supervised by the engine | `acanthus-services::kit` |

---

## 6. Technical risks and decisions

- **Video (decided: FFmpeg, minimal).** A purpose-built pipeline: demux/decode on a worker thread, upload frames as textures, loop/mute/seek; hardware decode through VA-API where available. Zero-copy dmabuf import is a stretch goal through wgpu-hal escape hatches; do not switch to raw `ash` on your own, raise the trade-off with the user first (E4.14).
- **Damage and present path.** wgpu 30 presents through Vulkan WSI without incremental present; E4.19 decides between a raw hook and documented full commits after measuring.
- **Blur cost.** Separable Gaussian on layer textures with a hash-keyed cache; popups with blur backdrops are the heavy case; the budget check lives in E7.9.
- **Hybrid GPU.** Pick the adapter explicitly (power preference option on `App`); never assume one GPU.
- **Plugins.** Native same-toolchain dynamic libraries (D2, decided 2026-10-27, `TOOLCHAIN.md` §2 and §4): works in a spike; panics are contained and quarantined, hard crashes and freezes end the process and are blamed so the supervisor restarts without the offender (Quickshell trust model, no sandbox).
- **Single-threaded UI state.** `Signal` is thread-local and `!Send`; background work marshals results through calloop channels. A panic in one module takes down the process, hence the panic hook, the restart unit and the separate lock binary.

---

## 7. Testing the engine

| Layer | Method |
|---|---|
| reactive, animation | plain unit tests with exact numbers, deterministic `tick(dt)` |
| layout, input | `Scene` tests: build nodes, `set_viewport`, `frame()`, assert geometry, drive pointer/keys, assert signals (`acanthus-scene/src/scene.rs` tests are the template) |
| rendering | headless GPU pixel tests (skip with a message without an adapter), golden PNGs with tolerance for complex visuals (regenerate with an env var) |
| services | trait-abstract the backend and test state machines against fakes; run a private `dbus-daemon --session` where practical |
| widgets | view tests in the widget crate with a rig; behaviour at the pixel positions users click |
| Wayland | manual only when the user asks; `wlrctl` generates pointer events, `hyprctl dispatch movecursor` does not |
| framework | module, action and popup-manager tests with fake hosts; examples must build and their tests pass in `check.sh` |
| layering | `scripts/check-layers.sh` |

---

## 8. Budgets (performance principles)

- Idle is 0 % CPU: no timers without a visible consumer, no frames without a change (the loop blocks with no timeout when idle).
- Clock-only change on a bar: one text layout and one glyph run upload, under 0.3 ms CPU per frame on the reference machine (AMD Renoir).
- Hover animations at the output refresh rate, paced by frame callbacks, never by busy timers.
- No per-frame heap allocation in steady state (display list and instance buffers reuse capacity).
- First frame under 150 ms target; `FontSystem::new()` (fontconfig scan, about 1.5 s) is the main cost: load lazily or on a worker.
- GPU memory: R8 atlas pages for masks, RGBA only for colour glyphs, layer textures freed when invisible, one shared context per format.
- Incremental rebuild of a one-line change in a leaf crate under 5 s (mold, line-tables-only; measured 3.6 s for the shell crate on 2026-10-26).
