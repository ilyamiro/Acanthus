# acanthus-wayland

**Parent context:** `../../AGENTS.md`
**Generated:** 2026-10-22 · **Updated:** 2026-10-22

## Purpose
Platform layer: owns the calloop event loop, sctk Wayland state, layer-shell surfaces (one wgpu surface each), pointer and keyboard input (xkbcommon, key repeat), cursor shape, frame pacing and timers.

## Key Files
| File | Description |
|------|-------------|
| `src/app.rs` | `App` (`layer_surface`, `every`, `after`, `run`), `State` with sctk handlers; loop timeout is `None` when idle, 16 ms while animating |
| `src/surface.rs` | `LayerConfig` builder, `Role` (layer or popup), `SurfaceWindow` (configure, scale, draw, frame callback) |
| `src/output.rs` | `OutputInfo` (id, name, make, model, position, size, scale, `WlOutput`) |
| `src/lazy.rs` | `LazyGate`: create-on-open, destroy-after-linger state machine behind `App::per_output_lazy` |
| `src/popup.rs` | `PopupConfig` / `PopupEdge` for `App::popup` (xdg_positioner values) |
| `src/window.rs` | `WindowConfig`, `WindowState`, `WindowCommand`, `Decorations`, `ResizeEdge`, `resolve_size` for `App::window` (xdg_toplevel) |
| `src/lock.rs` | `LockConfig`, `LockPhase`, pure `LockMachine` and `reconcile` behind `App::lock` (`ext-session-lock-v1`) |
| `src/idle_inhibit.rs` | `zwp_idle_inhibit_manager_v1` glue behind `AppHandle::set_idle_inhibit` |
| `src/scale.rs` | `wp_fractional_scale_v1` and `wp_viewporter` glue, `buffer_size` |

## For AI Agents

### Working In This Directory
- Do not run binaries that create surfaces unless the user asks.
- Keyboard and cursor-shape paths have not been run live yet; surfaces need `KeyboardInteractivity::OnDemand` to get keys.
- Outputs: `App::outputs()` is a `Signal<Vec<OutputInfo>>`; `App::per_output(config, build)` keeps one layer surface per output. Popups: `App::popup(parent, PopupConfig, build)`; a grab needs a prior input serial. Scale: fractional when the compositor offers it (buffer scale 1 plus a viewport destination), else integer.
- Known gaps: no IME or system clipboard, session lock (`App::lock`) and toplevel windows (`App::window`) are wired but never run against a compositor (roadmap §3.1, §3.5). Popups, outputs and fractional scale have never run against a compositor.
- Other crates register calloop sources on this loop; `acanthus-io` is designed to be generic over `LoopHandle`.

### Testing Requirements
Pure helpers have unit tests (`scale.rs`, `popup.rs`); the rest is verified with `cargo build` and clippy.

## Dependencies
Internal: acanthus-core, acanthus-anim, acanthus-scene, acanthus-render. External: smithay-client-toolkit, wayland-client, raw-window-handle, pollster.
