# acanthus-core

**Parent context:** `../../AGENTS.md`
**Generated:** 2026-10-22 · **Updated:** 2026-10-22

## Purpose
Reactive runtime and basic value types shared by every other crate.

## Key Files
| File | Description |
|------|-------------|
| `src/reactive.rs` | Thread-local runtime: `Signal<T>`, `Effect`, `bind`, `batch`, `untrack`, global `epoch()`; binding-loop guard (100k runs) |
| `src/types.rs` | `Color` (hex/parse/lerp/premultiplied), `Rect`, re-export of glam `Vec2` |

## For AI Agents

### Working In This Directory
- `Signal::set` skips equal values; `set_forced` always notifies. Reads inside an Effect subscribe; use `get_untracked` / `untrack` to avoid it.
- Subscriptions are never pruned (known harmless extra re-runs).
- `epoch()` bumps on every signal change; `Scene::needs_frame` relies on it (global invalidation is a known limitation, roadmap §2.3).

### Testing Requirements
`cargo test -p acanthus-core` (10 unit tests: chains, batching, disposal, loops, epoch).

## Dependencies
External: slotmap, glam, tracing.
