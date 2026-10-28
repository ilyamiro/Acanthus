# Handoff: where Acanthus stands and what to do next

Last updated 2026-10-28 (repository created; documents only). Older notes: `archive/`. Master plan: `ROADMAP.md`; checklists: `ENGINE.md`.

## 1. Snapshot

| | |
|---|---|
| Progress | **41.1 %** toward engine v1 (ROADMAP §3: E0 0.80 + E1 13.00 + E2 8.85 + E3 6.80 + E4 2.00 + E5 5.40 + E6 0 + E7 4.28 + E8 0) |
| Current step | **P0 baseline (E0.1 to E0.4)**: the repository holds documents, skill, hook and tmux helper only; no crates yet |
| Source of the code | the frozen repository `~/Projects/serp-rust` at its last commit before this day's split (`git -C ~/Projects/serp-rust log -1`); its `crates/engine/*`, `examples/`, `xtask/`, `docs/engine/` are the baseline, about 1,900 tests in the whole workspace (record the engine share at E0.4) |
| Git | `main`, local only; no remote yet (ROADMAP D11). Commit without pushing until the user adds one |
| Commands | cargo only through `nix-shell --run '...'`; `dbus-daemon` for service tests comes from the host |

## 2. Done in the frozen repository (carried over by E0.1)

E1 and E2 complete; E3 mostly (app identity, de-shelling, `acanthus-app` with `Module`/`Slot`/`Actions`/`Popup`/`dismiss`, typed settings `Section`, supervision, GPU economy, module scaffold); E4 popups, fractional scale, animated images; E5 nine widgets; E7 three examples, versioning policy, licence, CI workflow; rustdoc with `#![deny(missing_docs)]` finished for `acanthus-core`, `-anim`, `-image`, `-io` (missing-docs counts in the others: color 541, app 1098; wayland about 90 public items; runtime, scene, render, services, widgets not measured). Detailed per-slice history stays in the frozen repository's `.agents/HANDOFF.md` and `.agents/archive/`.

## 3. Latest slices

### 3.1 Repository created 2026-10-28 (E0.5, 41.1 %)
- Engine split out of the shell repository on the user's instruction: separate roadmap (`ROADMAP.md`, phases P0 to P9, shell demand matrix, copy plan for Serpantinum), `ENGINE.md` with new milestones E0 and E8, `/acanthus` skill (three slices per call), Stop hook that types the skill named in `.agents/next-slice` (empty marker uses the default `acanthus`), `scripts/acanthus-tmux.sh`.

## 5. What to do next, in order

Standing rules: never stop to ask which slice to take; three slices per `/acanthus` call; every slice states its milestone item.

1. **E0.1 baseline import** (one `chore:` commit): copy as described in `ENGINE.md` E0.1 from the frozen repository, write the root `Cargo.toml`, regenerate `Cargo.lock`, `nix-shell --run 'cargo build --workspace --all-targets'` green. Commit even if E0.2 to E0.4 are still red; say which tests fail.
2. **E0.2 fixtures** (copy test fonts and matugen fixtures into `assets/` with licence notes, one helper per crate) and **E0.3 scripts** (`check.sh`, `check-layers.sh` for a single layer, delete `layers.allow`).
3. **E0.4 fresh-clone proof** and CI; record the test count. Then P1: E7.5 rustdoc for `acanthus-color`, `-runtime`, `-wayland`, `-scene`, `-render`, `-services`, `-widgets`, `-app` in that order (one crate per slice, `#![deny(missing_docs)]` added when the crate is done), then P2 (ROADMAP §4).

## 6. Open questions

ROADMAP §7: D3 decided, D7 theme model, D8 feeds, D9 crates.io, D10 where Serpantinum lives after the copy, D11 the Git remote (the user creates the repository).
