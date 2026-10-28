# acanthus-app

App framework for building shells and apps (ENGINE.md E3.6).

- `Module<Cx>`: `name()` and `start(app, cx) -> Running`; `start_all` starts modules in order and reports each stage through a callback.
- `Provided`: typed map (`provide`, `get`, `require`) modules use to share controllers.
- `Actions` / `Verb` (`actions.rs`): named actions with aliases, exclusive groups, dismiss group, dispatch observers, `serve` onto an `IpcRegistry`.
- `Slot<T>`: lazily started shared service; `Slot::new` keeps it, `Slot::demand` stops it when the last `Rc` drops.

- `dismiss`: click-outside layer and input-region binding for popups on a full-output surface (`Spec`, `free_area`, `bind_input_region`, `attach`); the app passes a `reserved` strip.

- `Popup` (`popup.rs`): names, group, `is_open`, `open(arg)`, `close`, default `toggle`; `Actions::add` registers it and `closing_on_escape` wires Escape.
- `Actions::register_typed::<A: FromStr>`: typed argument (empty is `None`, unparsable is logged and skipped).

## Module contract

1. A module is one `impl Module<Cx>` per feature (bar, dock, a popup, a service wiring); `name()` is stable and shows up in logs and crash reports.
2. `start` builds surfaces and bindings and returns a `Running` value; everything that must live as long as the module (effects, memos, controllers, watches) goes into it, a dropped binding stops silently.
3. Modules share controllers through `Provided`: the producer calls `provide`, a consumer calls `require` (error names the missing type) or `get` for optional ones. Order in the module list is the dependency order.
4. Optional heavy services are `Slot`s: ask with `get_or_start`, hold the `Rc` only while the module is open, `demand` stops the service with the last handle (every `acanthus-services` handle stops its worker on drop, even one waiting on D-Bus or in a restart backoff).
5. A module that opens and closes UI implements `Popup` and registers with `Actions::add`; Escape, groups, click-outside (`dismiss`) and IPC come from the registry, the view never matches `Key::Escape` itself.
6. Settings are read through `Config::section::<T>(path)`; consume fields with `Section::select` so a sibling edit does not re-run the module. Settings spread over several paths or in legacy layouts use `Config::section_with(paths, shape)`; tolerant fields (QML truthiness, numeric strings, blank text as unset) use `acanthus_runtime::lenient` with `deserialize_with`.
7. Each module has a model (pure data and maths), a controller (signals and actions), a host (surface wiring) and a view (nodes); tests drive the model and controller without a GPU, view tests use the scene only.

- `testing` (`Headless`, `Script`, `Run`, `Image`): offscreen render through the real renderer, replay of `"360,90@500 click@800 key:Escape@1200 stall:600@300"` on a virtual 60 Hz clock, PNG output and a JSON frame log; skips (returns `NoAdapter`) without a Vulkan adapter, lavapipe is enough.

Scaffold: `cargo xtask new-module <name> [--dir <parent>]` (default parent `src`) writes `mod.rs`, `model.rs`, `controller.rs` (over a `Host` trait), `host.rs`, `view.rs` and `tests.rs` with two passing tests; add `pub mod <name>;` and a `Module` impl yourself.

Depends on `acanthus-render`, `acanthus-wayland`, `acanthus-io`, `acanthus-scene`, `acanthus-anim` and `acanthus-core`.
