# acanthus-color

**Parent context:** `../../AGENTS.md`
**Generated:** 2026-10-22 · **Updated:** 2026-10-22

## Purpose
Material You colour science without external binaries. Mechanical port of the `material-colors` crate (MIT OR Apache-2.0, same algorithm matugen uses) plus `generate.rs` with the matugen glue (scheme types, source colour ranking, smart-mode detection). The algorithm files are kept textually close to upstream on purpose: bit-exact output matters.

## Key Files
| File | Description |
|------|-------------|
| `src/generate.rs` | `SchemeType`, `ranked_source_colors`, `scheme`, `roles`, `hex`, smart-mode maths |
| `src/hct/`, `src/color.rs` | HCT, CAM16, solver, colour spaces (`mul_add` calls are deliberate) |
| `src/quantize/`, `src/score.rs` | Wu + WSMeans (Celebi) quantiser and scoring |
| `src/dynamic_color/`, `src/scheme/` | Dynamic scheme roles and the 9 variants |
| `tests/golden_schemes.json` | Output of the real matugen 4.2.0 for 4 colours x 9 types x dark/light |

## For AI Agents
- Do not "clean up" the numeric code: reordering float operations changes results. Compare against matugen (`SERP_COLOR_GOLDEN=<file> cargo test -p acanthus-color --release`) after any change.
- The image side (decode, resize) lives in `acanthus-runtime::colorgen`, not here.
- Add a `NOTICE` crediting material-colors and matugen before release.

### Testing Requirements
`cargo test -p acanthus-color`.

## Dependencies
External: indexmap.
