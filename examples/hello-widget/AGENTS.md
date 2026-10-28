# hello-widget

Desktop widget example (ENGINE.md E7.3): a card on the background layer with the time and the track MPRIS reports as active. Engine crates only (`scripts/check-layers.sh` enforces it for `examples/*`).

- `model.rs`: track label rules. `view.rs`: nodes over three `Signal<String>`. `main.rs`: `App`, `Clock::simple`, `Mpris` on the session bus, one `per_output` surface on `Layer::Background` with a negative exclusive zone.
- Run: `cargo run -p hello-widget` inside a Wayland session. Tests: `cargo nextest run -p hello-widget` (scene only, no GPU).
