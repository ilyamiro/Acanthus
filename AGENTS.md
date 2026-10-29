# Acanthus

**Created:** 2026-10-28 · **Progress:** about 58.2 % toward engine v1 (ROADMAP §3)

## Purpose
Acanthus is a standalone Rust toolkit for building Wayland shells, panels, lock screens, desktop widgets and floating apps: the Rust alternative to Quickshell plus the parts of Qt Quick they rely on (scene graph, layout, text, animation, shaders, input), with native system services as signals, an app framework, a widget library and a markup language (`.serp`) with hot reload. Rust 2024, wgpu on Vulkan, Wayland layer-shell. AGPL-3.0-or-later.

This repository is developed **on its own**. The first application, **Serpantinum** (`~/Projects/serp-rust`, the Rust adaptation of the QML shell `~/Projects/serpantinum`), is a frozen reference: it supplies the requirements (ROADMAP §5) and will be copied onto the finished engine afterwards (ROADMAP §8). Nothing here depends on it or names it in code.

The master plan, hard rules, milestone table (progress figure), phase order, shell demand matrix and slice protocol are in `.agents/ROADMAP.md`: read §0 to §6 before any task. The live checklist and the Quickshell/Qt parity matrix are in `.agents/ENGINE.md`; build, shipping and plugin decisions in `.agents/TOOLCHAIN.md`. The ordered slice queue and the latest session summary are in `.agents/HANDOFF.md`.

## Key files
| File | Description |
|---|---|
| `.agents/ROADMAP.md` | Master roadmap of the standalone engine |
| `.agents/ENGINE.md` | Contract per crate, parity matrix, milestone checklists E0 to E8, extension points, budgets |
| `.agents/TOOLCHAIN.md`, `ARCHITECTURE-REVIEW.md`, `ECOSYSTEM.md` | Build and shipping, evidence for the framework layer, ecosystem notes |
| `.agents/HANDOFF.md` | Snapshot and the ordered slice queue |
| `branding/` | Logo and wordmark |
| `.claude/skills/acanthus/` | The `/acanthus` skill (three slices per call) |
| `.claude/hooks/next-slice.sh` | Stop hook that restarts the skill in tmux when `.agents/next-slice` exists |
| `scripts/acanthus-tmux.sh` | Opens (or attaches) the tmux session running Claude Code for this repo |

Code: crates `acanthus-*`, `examples/`, `xtask/`, `docs/`, `assets/` (test fixtures), `scripts/check.sh`. `.agents/` and `.claude/` are local only (gitignored, never pushed).

## For AI Agents
- **Comments**: no explanatory comments in code (`//`, `/* */`, WGSL comments). Rustdoc (`///`, `//!`) is allowed and **required** on every public item (describe the item for its user, never narrate the implementation); `#![deny(missing_docs)]` per crate.
- **Commits**: conventional one-line prefix (`feat:`, `fix:`, `perf:`, `refactor:`, `style:`, `docs:`, `chore:`), commit after each finished task without asking, **never any AI attribution or Co-Authored-By line**. Pushing needs the user's say-so (the `/acanthus` skill carries a standing approval while it runs, once a remote exists).
- **No application knowledge in the engine**: no `serpantinum`, no `QS_`, no bar/dock concepts, no hard-coded app paths. Application identity (`AppId`), settings, assets, shader effects and IPC names are parameters.
- **Cargo only through `nix-shell --run '...'`**; quote globs in zsh. Look up crate APIs in `~/.cargo/registry/src/index.crates.io-*/` (wgpu 30, sctk 0.21, cosmic-text 0.19 differ from older memory).
- **Launching a GUI example** needs the user's say-so; tell them first, run under `timeout 20` in the background on a distinct namespace, never leave it running, report what happened.
- Workspace members are listed explicitly in `Cargo.toml`; no `crates/*` glob.
- All user-visible strings in examples and widgets go through the i18n layer; sentence case.
- Port Qt/Quickshell behaviour with fidelity first (durations, easing, spring constants), then optimise.
- After a task: update `ENGINE.md` checkboxes, ROADMAP §3, and `HANDOFF.md`.
