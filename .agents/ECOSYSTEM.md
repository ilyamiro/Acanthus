> **Absorbed 2026-10-27** into `ROADMAP.md` (engine track) and `ENGINE.md` E6 and E7. Kept for the reasoning; the checklists there are authoritative.

# M10 Ecosystem: markup language, apps and distro

Separate from `.agents/ROADMAP.md` on purpose. M10 is the long-term goal and is outside the "retire the QML shell" percentage (weight 0 in ROADMAP §14). Nothing here starts before the shell is at parity.

## Goal (user, 2026-10-25)

Finish the shell in Rust and perfect it, build more Rust apps on the same stack, grow one big ecosystem, then publish a distro.

## Decision: the markup is the UI layer of the whole ecosystem

This replaces the earlier "internal authoring format only" scope in ROADMAP §6 once M10 starts. Hot reload becomes a real feature, not only a developer convenience.

- [ ] Public-capable from the start: versioned syntax, stable set of native components, whitelist of expression functions.
- [ ] Whether to expose it to users is decided after the second app exists, not now.
- [ ] File extension `.serp` (short, unique, no collisions with `.qml`, `.ui`, `.slint`).
- [ ] Syntax close to QML so porting is mechanical.
- [ ] Expressions: small typed language, literals, property paths, arithmetic, comparison, ternary, calls into a whitelist (`t()`, `mix()`, `fmt_time()`); each compiles to a closure inside an `Effect`. No JavaScript engine.
- [ ] The Rust builder API stays the stable core; markup compiles to exactly the same builder calls and never replaces it. Performance-critical and logic-heavy widgets stay in Rust.
- [ ] Parser: hand-written recursive descent, errors with file:line:col shown in a dev overlay instead of crashing.
- [ ] Hot reload keeps signal state owned by services.

## Order of work

1. Shell reaches parity (M4 and M5 mostly done, then M6 to M9 as planned).
2. Build the next app or two with the Rust builder API only. Collect the repeated patterns. A language designed from one app fits only that app.
3. Design the markup from those patterns, then build it.
4. Tooling: dev overlay, `tree-sitter` grammar for editors, live-preview command, language server later.
5. Shared component library crate for all apps (theming, buttons, lists, popups).
6. Decide user exposure, theming surface and third-party apps.
7. Distro: packaging, session, defaults, installer, update story. Separate project with its own plan, written when step 6 is done.

## Risks

- Scope: a language, an app framework and a distro are three large projects. The shell is the only one with a finish line right now. Protect shipping the full shell at parity first.
- Designing the language too early from the shell alone.
- Tooling is most of the effort, not the parser.
- Exposing the format publicly freezes it; version it before exposing it.

## Progress

Not started. 0 %. No weight in the overall percentage until the user assigns one.
