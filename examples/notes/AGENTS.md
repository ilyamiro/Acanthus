# notes

Floating-window example (ENGINE.md E7.4): a note list (`list_view`), a wrapping text editor (`text_area`), a pinned checkbox and new/delete buttons in one `xdg_toplevel`. Engine crates only (`scripts/check-layers.sh` enforces it for `examples/*`).

- `model.rs`: `Note`, `title`, `pinned_first`. `controller.rs`: list, selection, editor text kept in step with the selected note. `view.rs`: nodes. `main.rs`: `App::window`, `I18n` from `i18n/` (override with `NOTES_I18N`). Notes live in memory only.
- Run: `cargo run -p notes` inside a Wayland session. Tests: `cargo nextest run -p notes` (scene only, no GPU).
