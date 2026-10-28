# hello-bar

Smallest Acanthus app (ENGINE.md E7.1): a top bar with a clock and the battery level. Depends on engine crates only (`scripts/check-layers.sh` enforces it for `examples/*`).

- `model.rs`: pure label maths. `view.rs`: nodes over two `Signal<String>`. `main.rs`: `App`, `Clock`, `UPower`, one `per_output` surface.
- No `Runtime`, no settings file: the example shows the layers one at a time (core signals, scene, wayland app, a clock, one D-Bus service).
- Run: `cargo run -p hello-bar` inside a Wayland session. Tests: `cargo test -p hello-bar` (scene only, no GPU).
