# acanthus-runtime

**Parent context:** `../../AGENTS.md`
**Generated:** 2026-10-22 · **Updated:** 2026-10-22

## Purpose
Config, i18n, theme, time and scaling services, wired together by `Runtime::start` and exposed over IPC. Reads `assets/` and `config/` from this repository (`SERPANTINUM_ASSETS` / `SERPANTINUM_DIR` override).

## Key Files
| File | Description |
|------|-------------|
| `src/config.rs` | `Config`: user file over bundled defaults, leaf `Signal`s, diffing hot reload, debounced atomic writes, `error()` for invalid JSON |
| `src/i18n.rs` | `I18n`: `t`, `tr`, `t_args`, RTL, language files watched, `system_language` |
| `src/qt_format.rs` | Qt `formatDateTime` pattern engine + language to chrono `Locale` mapping |
| `src/clock.rs` | `Clock` (port of DateTime.qml), wall-clock-aligned timer |
| `src/theme.rs` | `Palette` (22 roles), `Theme`, `ThemeService` (presets, matugen colours, icon font) |
| `src/matugen.rs` | `Matugen` request queue over a `MatugenRunner` trait, `to_md3` |
| `src/font_catalog.rs` | Scan of bundled and user font dirs |
| `src/scaler.rs` | Per-screen UI scale from `display.monitors.<name>.scale` / `general.uiScale` |
| `src/runtime.rs` | `Runtime` bundle and IPC targets `theme`, `fonts`, `matugen`, `config` |
| `src/paths.rs`, `src/json.rs` | XDG paths; dotted-path JSON helpers |

## For AI Agents

### Working In This Directory
- Settings are `serde_json::Value`; unknown keys must survive a write. Read with `Config::get::<T>(path, default)`.
- Reproduce QML quirks on purpose (see `.agents/archive/ROADMAP-legacy.md` §2.4 and `.agents/SHELL.md` §7) and write a test that names the quirk.
- Tests that call `Runtime::start` build a `FontSystem` (slow); keep them few.

### Testing Requirements
`cargo test -p acanthus-runtime` (59 tests).

## Dependencies
Internal: acanthus-core, acanthus-io, acanthus-scene (Fonts). External: serde_json, chrono (unstable-locales), cosmic-text (fontdb), calloop.
