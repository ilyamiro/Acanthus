---
name: acanthus
description: Continue building the Acanthus engine: read the handoff and roadmap, do the next three slices, verify, commit, push, then update the roadmap and handoff.
---

Read `.agents/HANDOFF.md` (sections 1 and 5 matter most: §5 is the ordered slice queue), `.agents/ROADMAP.md` §0 to §6 (vision, hard rules, milestone table in §3, phase order in §4, shell demand matrix in §5, slice protocol in §6), then only the checklist section of the slice in `.agents/ENGINE.md`, using `offset`/`limit`. Other background: `.agents/TOOLCHAIN.md` (build, shipping, plugins), `.agents/ARCHITECTURE-REVIEW.md`, `.agents/ECOSYSTEM.md`; older notes in `.agents/archive/`: open them only when you work on that area. Do the next slice, commit, push, and update the docs once at the end.

This repository is the engine only. Serpantinum (`~/Projects/serp-rust`, the QML shell adaptation) is a frozen reference and the future consumer; it is read for requirements (`.agents/ROADMAP.md` §5), never edited from here, never depended on. Engine code never names `serpantinum` or `QS_` and never imports an application.

Scope of a slice (one coherent unit: an engine feature, a widget, a platform protocol, a language stage):
- Implement the slice fully: logic, tests that run without any application, rustdoc on every public item you add or change, and a live check where the machine allows one (examples are launched only with the user's say-so).
- Before starting a slice, look for ways to do better than Qt or Quickshell while keeping their observable semantics: efficiency, rendering quality, a cleaner or more organic structure that is easier to support. Do not force a change; when one costs next to nothing, include it and mention it in the report. Qt fidelity of visible behaviour (durations, easing, spring constants) comes first.
- **Three slices per call (standing user instruction):** each `/acanthus` call does three slices in a row from the queue in HANDOFF §5, each with its own verification, commit and push, docs update and percentage recomputation. Do not stop after the first or second unless blocked. Report all three at the end.
- Large milestones are done slice by slice. Do not mark a checkbox or raise a percentage for work that is only partly done. If something essential is blocked, say so plainly.
- Reuse existing helpers and the module recipe (`crates/acanthus-app/AGENTS.md`) instead of rewriting them.

Autonomy (standing instruction from the user): never stop to ask which slice or approach to take. All open roadmap items must be done eventually, so pick the order yourself within the phase order of ROADMAP §4, choose the cleanest design, write the plan into the ROADMAP and HANDOFF, and carry it out. Large churn is not a reason to skip an item. Ask the user only when something is truly blocked, needs their eyes (a visual checkpoint) or is a decision listed in ROADMAP §7.

Rules that apply every time:
- Follow `AGENTS.md` and the ROADMAP §1 hard rules (no explanatory comments, rustdoc mandatory on public API, no AI attribution or Co-Authored-By lines in commits, no hard-coded paths, cargo only through `nix-shell --run`).
- Verify with `scripts/check.sh <crate>` while working (fast) and `scripts/check.sh` (whole workspace) once before committing.
- Commit with a conventional one-line prefix. Running this skill is the user's standing approval to push to origin once a remote exists (the remote is `origin`, ROADMAP D11).
- After the work, update the checkboxes in `ENGINE.md` and the table in ROADMAP §3 once, and the HANDOFF snapshot (test count, what was done in a short section, next steps). Keep the new HANDOFF section under about 15 lines and move older sections to the archive when it grows. Commit and push that too.
- Report the slice reached, how it was verified, the new overall completion percentage and the next milestone.
- Percentage: after every slice recompute the percentage with the method in ROADMAP §3 ("Counting rules"), and update the milestone row, the headline figure, `AGENTS.md` and the HANDOFF snapshot to the same number. Count finished items against total items for the milestone you touched, give partial credit only for tested and wired sub-tasks, show the arithmetic (weight x done, summed), never round up, lower the figure when scope grows or something turns out wrong. If a slice changes no item count, say the percentage is unchanged and why.

Last step, only after all three slices are committed, pushed and reported, and only when nothing is blocked and no visual checkpoint or user decision is pending: run `echo acanthus > .agents/next-slice` as the very last tool call, then end your turn with the report. A Stop hook (`.claude/hooks/next-slice.sh`) sees the marker, waits 5 seconds, and types `/clear` and the skill named in the marker into the tmux pane to start the next three slices with an empty context. If a slice is blocked, partly done, or needs the user, do not create the marker; say why instead.
