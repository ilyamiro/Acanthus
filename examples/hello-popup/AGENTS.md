# hello-popup

Second Acanthus example (ENGINE.md E7.2): a greeting card on a lazy overlay surface. Engine crates only.

- `controller.rs`: `Controller` implements `acanthus_app::Popup` (names `greeting`/`hello`, group `stack`), state is two signals.
- `view.rs`: the card, Escape closes through `closing_on_escape`. `main.rs`: `Actions::serve` on IPC target `main` method `handleCommand`, `per_output_lazy` bound to the open signal.
- `main.rs` also binds Super+G (`Keybindings`: compositor or portal) and `hello-popup activate toggle-greeting` calls the IPC `keys.activate`.
- Try: `cargo run -p hello-popup`, then send `{"target":"main","method":"handleCommand","args":["open","greeting","Ada"]}` to `$XDG_RUNTIME_DIR/hello-popup.sock` (or `close`, `toggle`).
- Tests: `cargo test -p hello-popup` (actions, controller and view, no GPU).
