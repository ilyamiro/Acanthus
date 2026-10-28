# Changelog

All notable changes to the Acanthus engine crates. The policy is in `docs/engine/versioning.md`.

## Unreleased

### Added

- Engine crates `acanthus-core`, `acanthus-anim`, `acanthus-color`, `acanthus-image`, `acanthus-scene`, `acanthus-render`, `acanthus-wayland`, `acanthus-io`, `acanthus-runtime`, `acanthus-services`, `acanthus-widgets` and `acanthus-app`, version `0.1.0`, not released yet.
- `Node::animate(Prop, Motion)`: QML `Behavior on <property>` for position, size, opacity, scale, rotation, colour, radius and border.
- `Tokens` (motion, spacing, radii) on `Theme`, `SettingKeys::reduced_motion` and reduced motion in `acanthus-anim` (`set_reduced_motion`, `reduced_motion`, `FAST`/`BASE`/`SLOW`).
- `cargo xtask api [--check]` and the per-crate public API lists in `docs/api/`, checked by `scripts/check.sh`.
- Examples `hello-bar`, `hello-popup` and `hello-widget`.
