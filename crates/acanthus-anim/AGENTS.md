# acanthus-anim

**Parent context:** `../../AGENTS.md`
**Generated:** 2026-10-22 · **Updated:** 2026-10-22

## Purpose
Qt-faithful animation engine on top of acanthus-core signals.

## Key Files
| File | Description |
|------|-------------|
| `src/easing.rs` | `Easing` with Qt's exact formulas (incl. OutExpo x1.001, InOutBack x1.525, `OutBack(s)`) |
| `src/animations.rs` | `Animation` trait; `Tween`, `Spring` (QSpringAnimation: fixed 16 ms steps), `Pause`, `Action`, `Sequential`, `Parallel`, `Loop` |
| `src/behavior.rs` | `Animated<T>` + `Motion`: the QML `Behavior on x` equivalent |
| `src/driver.rs` | Thread-local driver: `start`, `tick(dt)`, `is_active`, `AnimHandle` |
| `src/animatable.rs` | `Animatable` trait (f32, Vec2, Color) |

## For AI Agents

### Working In This Directory
- Do not change curve or spring maths without checking qtdeclarative; users tuned values by feel.
- The app loop calls `tick(dt)` only while `is_active()`; infinite animations keep frames flowing, so stop them when their item is hidden.
- Missing: `running` signal, `alwaysRunToEnd`, finished callbacks, State/Transition (roadmap §3.4).

### Testing Requirements
`cargo test -p acanthus-anim` (12 deterministic tests via `tick`).

## Dependencies
Internal: acanthus-core.
