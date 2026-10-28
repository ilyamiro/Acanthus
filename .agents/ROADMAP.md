# Acanthus roadmap: from baseline to engine v1

Written 2026-10-28. This is the master plan of the standalone engine. The live checklist is `ENGINE.md`, the next slices are in `HANDOFF.md` §5, build and shipping decisions in `TOOLCHAIN.md`.

## 0. What this project is

**Acanthus** is a toolkit for building Wayland shells and apps in Rust without Qt or Quickshell. It replaces Quickshell (windows, services, IPC, hot reload) and the parts of Qt Quick that shells use (scene graph, layout, text, animation, shaders, input), and adds a widget library, an app framework and an authoring language.

**Finish line, engine v1** (definition in `ENGINE.md` §1): a developer holding only the crates, `docs/` and the examples can, in Rust and in `.serp` markup, with hot reload, build a bar on any edge, popups and menus, a lock screen, a desktop widget and a floating-window app, use native services as signals, get theming, i18n, settings, IPC, supervision and crash handling by declaring a module, use a theme-driven widget library, and run it all with a CLI and an editor grammar. **And** the consumer-readiness probes of milestone E8 pass: every category of application that Serpantinum contains has been built against the engine alone.

**Why a separate repository.** The engine used to live beside the shell (`~/Projects/serp-rust`, crates `crates/engine/*`). The shell shaped the engine too much and slowed both. Acanthus is now developed on its own, finished first, and Serpantinum is copied onto it afterwards (§8). The shell repository is frozen as a reference; its engine crates are the **baseline** that milestone E0 imports.

**Not goals** unless the user adds them: accessibility tree, HDR and colour management, particles, web content, a JavaScript engine, liquid glass effects, a custom file picker (file choosing goes through the XDG portal; Ski, `~/Projects/Ski`, is the default backend).

Decisions already made (do not re-open unless asked): Rust 2024; wgpu on Vulkan (not raw ash); FFmpeg for video, minimal pipeline; no JavaScript; hybrid authoring (the Rust builder API is the stable core and markup compiles to the same calls); AGPL-3.0-or-later; name Acanthus, CLI `acanthus`, markup extension `.serp` (confirmed with the user when E6.4 starts); plugins as native same-toolchain dynamic libraries with containment (`TOOLCHAIN.md` §4).

---

## 1. Ground rules for agents

### 1.1 Hard rules (user decisions; they override convenience)
1. **No explanatory comments in code.** No `//`, no `/* */`, no WGSL comments. Rustdoc `///` and `//!` is required on every public item and allowed nowhere else; `#![deny(missing_docs)]` per crate. `scripts/check.sh` rejects every other comment.
2. **Examples that open windows** are launched only when the user says so (tell them first, background, `timeout 20`, distinct namespace, `exclusive_zone(0)`, never left running, report what happened).
3. **Commits**: conventional one-line prefix, commit after each finished task, **no AI attribution or Co-Authored-By lines, strict**. Pushing needs the user's say-so unless the `/acanthus` skill is running.
4. **No hard-coded paths and no application names.** Directories and environment overrides derive from an `AppId { name, env_prefix }` the application passes in. Stay compositor-agnostic (Hyprland, Sway, Niri, MangoWC): probe capabilities, never assume.
5. **Full i18n** for user-visible strings in widgets and examples; sentence case.
6. **Report milestones** unprompted: what was reached, how it was verified, the new percentage with arithmetic, the next milestone.
7. **Engine APIs are written for a second consumer.** No application constants, no palette role names only one app uses, no bar/dock/popup knowledge beyond generic mechanisms. Anything configurable is a parameter, a trait or a builder option. Every feature has tests that run without any application and, when practical, an example.
8. **Qt/Quickshell fidelity first** for observable behaviour (easing formulas, `SpringAnimation` stepping, synchronous bindings), then optimise; improvements that keep semantics are welcome.
9. **Moves are their own commits** (`refactor:`), no behaviour change, identical tests.
10. **Public API discipline**: from E7.6 on the stable surface is listed per crate; breaking it follows `docs/versioning.md`.

### 1.2 Environment
Cargo only through `nix-shell --run`; toolchain pinned by `rust-toolchain.toml` and `shell.nix`; new system libraries go into `shell.nix`. Read the sources of Qt (`qt/qtdeclarative`), Quickshell (`git.outfoxxed.me/quickshell/quickshell`) and the external crates before porting or guessing.

### 1.3 Definition of done for a slice
`scripts/check.sh <crate>` while working, `scripts/check.sh` before committing (fmt, layer/app-name check, build, deny, features, doc with warnings denied, clippy as errors, nextest + doc tests, no-comments). New behaviour has tests (`ENGINE.md` §7). Every new or changed public item has rustdoc. Checklist, §3 here and `HANDOFF.md` updated once at the end.

---

## 2. Repository layout

Target (flat; code arrives with E0):

```
Acanthus/
  Cargo.toml  rust-toolchain.toml  shell.nix  rustfmt.toml  deny.toml  LICENSE.md  NOTICE  CHANGELOG.md
  crates/  acanthus-{core,anim,color,image,scene,render,wayland,io,runtime,services,widgets,app}
           acanthus-markup  acanthus-cli  acanthus (facade, E7.11)       (the last three are new)
  examples/  hello-{bar,popup,widget} ... and the E8 probes
  xtask/     docs/ (guides, versioning)    assets/ (test fixtures only)    scripts/    branding/
  .agents/   .claude/   .github/
```

Layering inside the repository: `core < anim, color, image < scene < render < wayland < runtime/io/services < widgets < app < markup < cli/facade`. A crate never depends on a later one; examples depend on crates only; `scripts/check-layers.sh` (rewritten for a single-layer repo in E0.3) fails on any manifest path leaving the repository and on the strings `serpantinum` or `QS_` in sources, assets or docs, except the migration guide (E8.10) and `ROADMAP.md`/`HANDOFF.md`.

---

## 3. Milestones and progress

**Progress = sum of weight x done % / 100**, one decimal, truncated, never rounded up.

| ID | Milestone | Weight | Done | State |
|---|---|---:|---:|---|
| E0 | Standalone baseline: import of the engine crates from the frozen shell repo, fixtures, scripts, CI, fresh-clone proof (5 items) | 4 | 20 % | **NEXT** (E0.5 tooling done 2026-10-28) |
| E1 | Rendering and scene core (finished) | 13 | 100 % | gaps are items in E4 |
| E2 | Runtime and native services (finished) | 9 | 98.3 % | |
| E3 | App framework and build foundation (16 items) | 8 | 85.0 % | 13.6 of 16 |
| E4 | Platform surfaces and render completeness (21 items) | 14 | 14.3 % | 3 of 21 |
| E5 | Widget library (20 items) | 12 | 45.0 % | 9 of 20 |
| E6 | Authoring and host: contract, tokens, `.serp` language, hot reload, CLI, editor support (13 items) | 20 | 0 % | |
| E7 | Docs, proof and release engineering (14 items) | 10 | 42.9 % | 6 of 14 |
| E8 | Consumer readiness: probes of every application category, migration guide, budgets, v1.0.0 (12 items) | 10 | 0 % | |

**Overall: 41.1 %.** Arithmetic (2026-10-28): E0 4 x 1/5 = 0.80; E1 13 x 100 % = 13.00; E2 9 x 98.3 % = 8.85; E3 8 x 13.6/16 = 6.80; E4 14 x 3/21 = 2.00; E5 12 x 9/20 = 5.40; E6 0; E7 10 x 6/14 = 4.28; E8 0; sum 41.13, truncated 41.1.

The earlier combined figure (engine and shell in one repository) was 49.1 %; the two are not comparable: the engine is now measured alone, with the language weighted at 20, a new baseline milestone and a new readiness milestone.

Counting rules:
- A milestone's % = finished items / total items. A finished item counts 1; a partly finished one counts only the share of its sub-tasks that are done, tested and wired, with the credit written in the checklist; an unstarted one counts 0.
- Count only what is tested and wired. An item that failed a live look does not count.
- Adding items lowers the milestone's % immediately (say so in the report); removing an item needs a reason in the commit.
- Keep this table, `AGENTS.md` and `HANDOFF.md` §1 at the same number.

---

## 4. Phase order

The order is the plan; reorder only with the user's say-so. Within a phase pick the cleanest order yourself.

| Phase | Content | Gate |
|---|---|---|
| **P0 Baseline** | E0.1 to E0.4 (import, fixtures, scripts, CI) | fresh clone passes `scripts/check.sh`; test count equals the frozen repo's engine share |
| **P1 Docs of what exists** | E7.5 rustdoc for the remaining crates (color, runtime, wayland, scene, render, services, widgets, app), then guides | `#![deny(missing_docs)]` everywhere, `cargo doc` clean. Done early because every later slice must document its own API |
| **P2 Framework** | finish E3 (E3.6 hub follow-ups, E3.7 keybinding source, E3.9 settings leftovers, E3.10 logical units, E3.15 flake/crane/features/xtask) | E3 at 100 % |
| **P3 Authoring foundations** | E6.1 component contract, E6.2 node-level animate, E6.3 tokens; E7.6 public API list | contract used by all new widgets |
| **P4 Platform surfaces** | E4 in this order: E4.4 menus, E4.5 toplevel, E4.6 session lock, E4.8 clipboard, E4.9 shortcuts, E4.7 screencopy, E4.10 DnD, then render completeness (E4.11 to E4.13, E4.15 to E4.21), E4.14 video last | E4 at 100 % |
| **P5 Language** | E6.4 spec v0 (written from `examples/` and the E5 widgets), E6.5 parser, E6.6 expressions, E6.7 compiler, E6.8 hot reload, E6.9 CLI, E6.10 grammar, E6.13 host | markup builds the three hello examples identically to the Rust versions |
| **P6 Widgets** | E5.10 to E5.20, written against the contract and exposed to markup | E5 at 100 % |
| **P7 Release engineering** | E7.4, E7.9 to E7.14 (floating-window example, benchmarks, plugins, crash handler, containment, baselines), E6.11/E6.12 | E6 and E7 at 100 % |
| **P8 Consumer readiness** | E8.1 to E8.11: probes in markup, performance acceptance, migration guide | every probe passes its acceptance |
| **P9 Release** | E8.12: `v1.0.0` tag, API list frozen, docs book, handoff statement "ready for Serpantinum" | the user confirms; then §8 starts |

P1 to P4 may interleave slice by slice when an item blocks another (prerequisites in ENGINE.md items); P5 does not start before P3 and the P4 surfaces its examples need exist, because a language designed too early fits only the examples that exist.

---

## 5. Shell demand matrix (what Serpantinum needs; the engine must cover all of it)

Source: the frozen shell repository's module list. Each row is a category the engine has to support with generic mechanisms; the probe is the E8 item proving it.

| Category in Serpantinum | Engine capability | Items | Probe |
|---|---|---|---|
| Bar on four edges, per output, faces with live data, autohide, exclusive zone, input regions | layer surfaces, `per_output`, services, widgets | E1, E2, E4.2 | E8.1 |
| Popups of ten kinds (media, calendar, volume, network, clipboard, launcher, syspanel, notifications centre) with open/close morph, exclusive groups, Escape, click-outside | `acanthus-app` popup manager, popup frame, dropdown, tabs, menus | E3.8, E4.4, E5.10, E5.13, E5.16 | E8.2 |
| Dock with hover cascade, running indicators, autohide edge trigger, context menus | layer surface, hit regions, `xdg_popup` | E4.4, E5 | E8.3 |
| Lock screen as its own crash-proof binary, PAM, fingerprint, clock, backdrop from the live screen | `ext-session-lock`, PAM service, screencopy, supervision | E4.6, E4.7, E3.11 | E8.4 |
| Wallpaper engine: images, video, transitions with shaders, per output | video node, shader effects, background layer | E4.14, E1 | E8.5 |
| Desktop widgets with a redactor (drag, resize, snap), custom drawn graphs | DnD, drag handlers, canvas, background layer | E4.10, E4.13, E5.20 | E8.6 |
| Settings and guide app (about 90 form pages, search, tabs, dialogs, number and time selectors, file choosing by portal) | toplevel windows, text editing, widgets, schema-driven settings, markup | E4.5, E4.16, E5.11 to E5.19, E3.9, E6 | E8.7 |
| Notifications server, OSD, tray (SNI + dbusmenu) | notification service, tray service, popup frame | E2, E5 | E8.8 |
| Audio visualiser (cava), equalizer curves, waveforms | canvas and shader effects, audio service | E4.13, E1.7 | E8.9 |
| Hot reload of settings, theme from wallpaper (Material You, matugen-exact), i18n in 23 languages, IPC, CLI parity | `acanthus-runtime`, `acanthus-color`, host and CLI | E2, E6.9, E6.13 | E8.10 |
| Memory and CPU at or below the QML shell (497 MB resident, 49 threads measured 2026-10-27) | budgets | E7.9, E7.13 | E8.11 |

If a Serpantinum feature cannot be expressed cleanly, the engine is wrong, not the shell: add the missing item to `ENGINE.md` (and say so in the report, since scope growth lowers the figure).

---

## 6. Slice protocol (one coherent unit per run, three per `/acanthus` call)

1. Read this file §0 to §6, `HANDOFF.md` §1 and §5, then the milestone section you work in.
2. Read the Qt/Quickshell sources for parity work, the current Rust APIs you touch and the external crate sources.
3. Implement the smallest complete slice with tests, rustdoc, no other comments.
4. Definition of done (§1.3). For moves: identical test count.
5. Update the checklist, §3, `AGENTS.md` and `HANDOFF.md` once at the end (HANDOFF sections stay under about 15 lines; older ones go to `archive/`).
6. Commit, push if a remote exists and the skill is running, report: slice reached, verification, new percentage with the sum, next slice.
7. Stop and ask only when truly blocked, when something needs the user's eyes, or for a decision in §7.

```sh
scripts/check.sh <crate>      # quick
scripts/check.sh              # whole workspace before committing
```

---

## 7. Open decisions (recommendation first)

| # | Decision | Recommendation | Blocks |
|---|---|---|---|
| D2 | Plugins | **Decided** (`TOOLCHAIN.md` §4); remaining work is E7.11, E7.12, E7.14 | none |
| D3 | When the language is designed | **Decided here:** after P3/P4 and the existing examples (§4), not after a shell exists | none |
| D7 | Theme model | engine owns tokens (colour roles, motion, spacing, radii, fonts) and colour generation; the Catppuccin-style role names (base, mantle, surface0, mauve ...) stay the default role set; apps supply presets, templates, defaults | E6.3 |
| D8 | User-defined feeds and daemons declared in settings | ask before starting | |
| D9 | Publish on crates.io | decide at E8.12 when the API is frozen | E8.12 |
| D10 | Where Serpantinum lives after the copy (new repo, or `serp-rust` rewritten in place) | recommend a new directory/repo that starts from the copy checklist in §8; ask at P9 | §8 |
| D11 | Git remote | **Decided 2026-10-28:** `git@github.com:ilyamiro/Acanthus.git` (origin); the `/acanthus` skill pushes to it | none |

---

## 8. After engine v1: copying Serpantinum onto Acanthus

Starts only when P9 is done and the user says so. Plan, to be refined then:
1. Create the application workspace (D10) depending on Acanthus by git tag.
2. Copy the data that is not code: `assets/`, `config/`, `compositors/`, `packaging/`, language files, themes, sounds, fonts, shaders (shader registration through the engine's effect registry).
3. Copy the shell crates (`serpantinum-audio`, `-effects`, `-services`, `serpantinum`) and re-point imports to the tagged crates; the frozen repository's tests are the safety net (about 1,800 tests, pixel-compare view tests).
4. Port module by module, markup first where the migration guide (E8.10) says it pays off; data directories and formats stay exactly as the QML shell wrote them so users migrate untouched.
5. Retire the frozen repository when parity of every module is shown.
Any engine gap found in step 3 or 4 becomes an engine item first (engine slice, then application slice), never an application workaround.
