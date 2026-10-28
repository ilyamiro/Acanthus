> **Status 2026-10-27:** the findings and the R1 to R3 order are now milestone E3 (and parts of E4 and E6) in `ENGINE.md`; the mapping from R items to E items is under E3 there.

# Architecture review, 2026-10-26 (after M5 slice 90)

Scope: the whole workspace at about 54 % of the port (126k lines of Rust, 1760 tests). The question asked by the user: now that the one-to-one port is half done, what will turn into a bottleneck once the Rust shell grows on its own (new features, plugins, the `.serp` markup with hot reload), and when should each thing be fixed. The checklist that comes out of this is milestone **M5R** in `ROADMAP.md` §16.

Every claim below was checked in the code on 2026-10-26; the numbers are greps or measurements, not impressions.

## What is good and should stay

- The per-module split `model.rs` (pure) / `controller.rs` (state machine over a `Host` trait) / `host.rs` (real services) / `view.rs` (scene) / `tests.rs` is consistent across all popups and is the reason 1760 tests exist. Keep it as the module contract.
- `kit::Ctx` with injected `Exec`, `Clock`, `Translator`, `Notifier` makes services testable without the system. Keep.
- `WeakHandle` discipline for calloop removal, the Quickshell path inheritance (§5b.0), the native colour generation and the audio daemon are solid.
- The reactive core is small and readable. The problems below are about what is missing around it, not about rewriting it.

## Findings, by how much they will hurt

### 1. GPU resources are per surface, and every popup is a permanently mapped full-screen surface (high, Wayland and memory)

Evidence:
- `acanthus-wayland/src/surface.rs::reconfigure` builds a `Renderer` per surface. `Renderer::new` compiles every pipeline (28 effect pipelines plus quad, glyph, image) and allocates its own 2048² RGBA glyph atlas (16 MB) and its own image textures (`acanthus-render/src/renderer.rs`).
- `main.rs` creates 12 layer surfaces **per output** through `App::per_output` (osd, traymenu, calendar, network, music, lyrics-picker, clipboard, launcher, dock, dockmenu, dock-exclusion, system), 11 of them anchored to all four edges, plus the bar hosts and the notification stack. They stay mapped while closed.
- No `damage_buffer` and no `set_opaque_region` call anywhere in `acanthus-wayland` or `acanthus-render`.

Why it hurts:
- Estimate for one 2560x1440 output: 11 full-screen swapchains of about 3 x 14.7 MB, plus about 14 glyph atlases of 16 MB, is in the region of 700 MB of VRAM before anything is open; double it for two monitors. Startup compiles the same pipelines about 14 times per output. Measure it (item R1.0) before arguing about the exact figure.
- A permanently mapped full-screen overlay surface, even fully transparent, sits above fullscreen games and video. Compositors then cannot use direct scanout for them, which costs latency and battery and can break tearing and VRR setups. It is also why every popup has to compute its own input region (67 `input_region` sites in the shell crate).
- Glyphs rasterised for the bar are rasterised again for every popup; an image shown in two popups is uploaded twice.

Fix:
- `acanthus-render`: split `Renderer` into a shared `GpuContext` (device, queue, pipelines, one glyph atlas, ideally the R8 mask atlas already on the §2.3 list, one image texture cache keyed by `ImageId`) and a light per-surface `SurfaceRenderer` (swapchain, globals, layer texture pool).
- `acanthus-wayland`: lazy surfaces. A popup surface is created when its controller opens and destroyed (or unmapped by attaching a null buffer) a moment after its exit animation ends. One `LazySurface` helper in the engine, driven by a `Signal<bool>`, so modules do not reimplement it.
- Where the visual allows it, size the surface to the popup's bounds (plus the animation margin) instead of the whole output; menus anchored to the bar (tray menu, dock menu) are candidates for `xdg_popup` with a grab, which gives click-outside-to-close and correct stacking for free (`App::popup` already exists).
- Pass damage (`damage_buffer` for the union of changed primitive rects) and an opaque region where the background is solid.

When: now. Each new popup adds another permanent surface, and the lock screen and wallpaper will add more per output.

### 2. `main.rs` is the composition root, the event bus and the module registry at once (high, extensibility)

Evidence:
- `fn main()` is about 1,880 lines (`crates/shell/serpantinum/src/main.rs:44`).
- Every bar face is wired by hand with a struct that repeats `theme`, `vertical`, `compact`, `scale`, `cross`, `active`, `cascade_finished`, `schedule`, `click_sound` (`main.rs:880-1060`).
- Settings are derived one by one with `derive(...)` and a `_xxx_binding` local to keep each binding alive (17 times for `config.value("bar")`).
- Cross-module actions are closures written in `main` and several are still stubs: `open_launcher: Rc::new(|| tracing::info!("launcher toggle requested"))`, `open_settings`, `toggle_launcher`.
- `main handleCommand` (`main.rs:703`) is one closure that knows every popup, and popup exclusivity is hand-coded pair by pair (opening the launcher hides the clipboard and the reverse). Nine `enum Ipc` / `enum Command` types repeat Toggle/Open/Close.

Why it hurts: every new module touches the same function in four places. A plugin cannot add a module without editing `main`, and a keybinding, a bar click, a markup file and IPC all reach the same action through different code paths.

Fix (one design, three parts):
- **`ShellModule` trait and a module registry.** Each module (bar, dock, launcher, music, …) implements `fn start(cx: &ModuleCx) -> Box<dyn Running>` and declares its surfaces, IPC methods, actions and settings section. `main` becomes: build `Hub`, build `ModuleCx`, start each registered module. Adding a module is one file plus one line in the registry.
- **Action registry.** Named, typed actions (`launcher.toggle`, `popup.open { name }`, `settings.open { page }`) registered by modules and invoked from anywhere: IPC (`qs ipc call` names map onto actions, so the QML targets keep working), bar clicks, keybindings and later markup. This replaces the `Rc<dyn Fn()>` callback fields between modules (189 `Rc<dyn Fn` in the workspace).
- **Popup manager.** One `Popup` trait (`open`, `close`, `toggle`, `is_open`, `exclusive_group`) and one manager that handles exclusivity, Escape, click-outside, keyboard focus and the lazy surface from finding 1. The nine Ipc enums and the pairwise hiding disappear.

When: now, before the next popup. It is mostly moving code, and the cost grows with each module.

### 3. Engine primitives live in the binary's `bar/` folder (high, layering)

Evidence:
- `bar/motion.rs` defines `Keep` (a reactive ownership scope), `Keep::derive` (a memo), `Eased` (Behavior), springs and loops. It is imported by 32 files outside `bar/`. `acanthus-anim::Animated<T>`, the engine's own Behavior, has 2 users; `Eased`/`keep.ease` has 262.
- `derive` exists three times (`main.rs:38`, `bar/motion.rs:82`, `acanthus-services/src/kit/settings.rs:9`).
- `darken(color, factor)` is copy-pasted into seven model files (`bar/{bt,sysmon,bat,kb,wifi,vol}/model.rs`, `syspanel/model.rs`); `lighten` lives in `bar/motion.rs`.
- Reusables (`toggle.rs`, `switch.rs`, `bar/button.rs`, the file picker, the marquee in `bar/media`) are in the binary; `bar::button` is used by 21 files outside `bar/`. ROADMAP §7 planned a `acanthus-widgets` crate; it was never created.
- `timing.rs` holds three duration constants; every other duration is a literal.

Why it hurts: the markup (M8/M10), a second app and plugins all need these as a library. Right now they cannot use them without depending on the shell binary. Two Behavior implementations also mean two sets of bugs.

Fix:
- `acanthus-core`: add `Memo<T>` (what `derive` does, as a type) and an ownership `Scope` (what `Keep` does), so `Node::own`/`Keep`/`_binding` locals collapse into one concept.
- `acanthus-anim`: move `Eased`, the spring and loop helpers in; delete or merge `Animated<T>` so there is one Behavior.
- `acanthus-core::Color`: `lighten`, `darken`, `mix`, `with_alpha` once.
- New crate `acanthus-widgets`: button and press feel, toggle, switch, slider, text field, marquee, list row, scroll handle, popup frame. Motion and spacing tokens (`FAST`/`BASE`/`SLOW`, radii) live in the theme, not in literals.
- Ergonomics: the pattern `keep.derive(target) → keep.ease(&target, ms, easing)` appears hundreds of times; a node-level `.animate(Prop::Color, Motion::BASE)` (QML `Behavior on color`) halves the view code and is exactly what the markup will need to compile to.

When: now, together with finding 2. It is mechanical and gets more expensive with every slice.

### 4. Build times are already 15 s for a one-line change (medium now, high later)

Measured 2026-10-26: touching `crates/shell/serpantinum/src/toggle.rs` and rebuilding takes 15.5 s; touching `acanthus-scene/src/node.rs` takes 13.6 s. The shell crate is 61,683 lines in one compilation unit, and the guide/settings app alone is 20,886 lines of QML still to port. The default linker is used.

Fix: the `acanthus-widgets` split from finding 3, then one crate per large module once the `ShellModule` trait exists (`serp-mod-dock`, `serp-mod-music`, …), so an edit recompiles one module and the link. Add `mold` (or `lld`) to `shell.nix` and `.cargo/config.toml`, and `debug = "line-tables-only"` for the dev profile. The linker change is a ten-minute task with a large return; do it first.

### 5. D-Bus services are one-shot threads with one connection each and no restart (medium, robustness)

Evidence:
- `platform/bridge.rs::spawn_worker` starts an OS thread with its own `async_io::block_on` executor per service; each service calls `bus.connect()` and opens its own D-Bus connection (MPRIS, UPower, NetworkManager, BlueZ, tray, notifications, polkit, power profiles, brightness, screensaver, …). There are 42 `thread::spawn` sites in non-test code.
- When the worker's stream ends or a call fails, the service sends `Unavailable` and stops (`platform/upower.rs:142-167`). Only tray, BlueZ, polkit, MPRIS and the screensaver watch name owners. Restarting NetworkManager, UPower or power-profiles-daemon leaves the shell showing stale or missing state until the shell restarts.
- `kit::Daemon` already supervises child processes with restart; D-Bus workers do not get the same treatment.

Fix: one async executor thread for all D-Bus work with one shared session and one shared system `zbus::Connection` (zbus multiplexes); a `Supervised` wrapper in `kit` that restarts a worker with backoff and on `NameOwnerChanged` for its well-known name, reusing the `Daemon` policy. Services become demand-driven: the `Hub` starts a service on first `get` and stops it when the last handle drops (today a started service runs forever).

When: before M6. The lock, polkit and idle paths must survive a daemon restart.

### 6. One panic takes down the bar, every popup and, after M6, the lock screen (high for M6)

Evidence: all UI state is `Rc<RefCell<…>>` on one thread (755 `borrow_mut()` calls); `Signal::update` holds a `borrow_mut` while its closure runs, so a closure that reads or writes the same signal panics. There is no panic hook that restarts anything. ROADMAP §3.1 says "keep the lock path minimal and panic-free" but does not say how.

Fix: make the lock screen its own small binary (`serpantinum-lock`) built from the same crates (scene, render, wayland, PAM controller), started by the shell over IPC or by the idle module. If it crashes, `ext_session_lock_v1` keeps the session locked and the shell restarts the lock binary; if the shell crashes, the lock is unaffected. For the shell itself: a supervisor (systemd user unit with `Restart=on-failure`, or the `serpantinum` CLI) restarts it, and the panic hook logs the module and the last action.

When: decide before M6 starts, because it shapes the lock module.

### 7. Rendering does full work for every change (medium, measure first)

Evidence: `Scene::frame` (`acanthus-scene/src/scene.rs:242`) re-runs layout for every node and rebuilds and compares the whole display list whenever any tracked signal in the scene changed. Any animation on a surface therefore re-lays out the whole surface every frame. A `Node` holds about 25 `Signal`s, each a separate `Rc` allocation, even for values that never change. The animation driver advances on wall-clock `dt` clamped to 0.1 s (`acanthus-wayland/src/app.rs:327`) rather than frame-callback timestamps.

Why it may hurt: fine for the bar; the guide/settings app (heavy forms, 40 Flickables) and desktop widgets will have thousands of nodes on one surface.

Fix, only after profiling a heavy scene: per-node dirty flags with relayout from the nearest layout boundary (fixed-size items are natural boundaries), display-list reuse per subtree, `Prop<T>` that is `Static(T)` until something binds it, frame-callback timestamps for the animation clock. Add a counter overlay (layout ms, primitives, frames per second) first so the decision is data-driven.

When: before the guide/settings app (M8).

### 8. Configuration is untyped JSON read at many places (medium)

Evidence: 33 `config.value("…")` calls, 17 of them for the whole `bar` object; 96 `.get("key")` string lookups and 98 `serde_json::Value` mentions in the shell crate. Any change under `bar.*` re-parses the settings of every bar face.

Fix: serde-derived typed settings per section (`BarSettings`, `DockSettings`, …), parsed once per file change, published as `Signal<Rc<Settings>>` with per-field selectors (`Memo`) so a face only re-runs when its fields change. The structs double as the settings schema the guide app (M8) needs to render forms and as validation with useful errors. File format and paths stay exactly the Quickshell ones (§5b.0).

When: with finding 2 (each module declares its settings section), at the latest before M8.

### 9. Scale is baked into every view at build time (medium, markup)

Evidence: about 1,600 places multiply a literal by the scale (`s(…)`, `* scale`); 82 `scale: f32` fields against 16 `Signal<f32>`. A scale change rebuilds the faces (`bar/frame/rebuild.rs`).

Fix: views write logical units; the scene applies the UI scale (with the same rounding as `Scaler.s`) during layout. Do this together with the markup design, since markup should never carry `* scale`. Fidelity check: compare pixel positions with the current output in tests before switching a module.

When: before or together with M8, module by module.

### 10. The markup needs a component contract that does not exist yet (medium, long term)

Each view is a function `build(scene, parent, Inputs)` with a module-specific `Inputs` struct of signals and callbacks. A markup file (and hot reload) needs to instantiate components by name, set props by name, bind expressions to them and rebuild a subtree while keeping state. Define the contract now, while there are only about 30 reusables: `trait Component { type Props: Props; fn build(cx, props) -> NodeId }` with `Props` derivable (named, typed, defaultable fields, each `Static` or bound), plus a registry by name. New reusables in `acanthus-widgets` follow it from day one; the `.serp` compiler later targets it. ROADMAP §15 says "do not design the language yet"; this is not the language, only the seam it plugs into.

### Smaller items

- Popups know bar internals: bar thickness, position and offsets are read in ten modules (`bar_offset`/`bar_thickness` greps). Publish one `BarGeometry` signal per output from the bar module and have popups read only that.
- `Kind` is a closed enum (`Item`, `Rect`, `Text`, `Image`, `Shader`); the quick-actions Draw tool and graphs need a custom paint node (a `Canvas` kind that records primitives from a closure).
- Diamond dependencies in `acanthus-core` run effects with half-updated inputs and run them twice (FIFO queue, no height ordering). QML has the same glitches, so parity is fine; a `Memo` with height-ordered flush fixes both if it shows up.
- Clipboard should move from `cliphist` + `wl-paste --watch` to `ext-data-control-v1` / `zwlr_data_control_v1` natively (already on the roadmap as "later").

## Order

1. **R1, now (before slice 91):** mold linker; measure VRAM and startup; shared `GpuContext`; lazy popup surfaces; `Memo` and `Scope` in core, one Behavior in `acanthus-anim`, colour helpers; `acanthus-widgets`; `ShellModule` registry, action registry and popup manager; `main.rs` reduced to the registry.
2. **R2, before M6:** shared D-Bus executor and connections with supervision and demand start; lock as its own binary; panic and restart policy.
3. **R3, before M8:** typed settings with selectors; profiling overlay and incremental layout and damage; scale in the scene; the `Component`/`Props` contract and per-module crates.

Each step keeps the full definition of done green and changes no visuals; the existing view tests are the safety net.
