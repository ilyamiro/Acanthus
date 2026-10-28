# Versioning, changelog and deprecation

This is the policy for the Acanthus engine crates (`acanthus-*`). The Serpantinum shell crates are an application and follow `version.txt`, not this policy.

## Versions

- All engine crates share one version, `workspace.package.version` in the root `Cargo.toml`, and are released together.
- The version follows semantic versioning. While the major version is `0`, a minor bump (`0.x.0`) may break the public API and a patch bump (`0.x.y`) never does. From `1.0.0` a breaking change needs a major bump.
- The public API is what the crate exposes through `pub` items that are documented (rustdoc is required on all of them, E7.5). Items marked `#[doc(hidden)]`, private modules and anything in `examples/` or the `xtask` crate are not part of it.
- Breaking changes are: removing or renaming a public item, changing a signature, adding a required trait method, adding a variant to an exhaustive enum or a field to a struct with public fields, raising the minimum supported Rust version, changing a Cargo feature that is on by default, changing a data file format that apps own (settings, caches, state) without a migration.
- Adding an item, a defaulted trait method, a Cargo feature or a `#[non_exhaustive]` variant is not breaking.
- Public enums that are expected to grow and public option structs are `#[non_exhaustive]` from the first release of the crate that exposes them.

## Changelog

- `CHANGELOG.md` at the repository root follows "Keep a Changelog": an `Unreleased` section on top, then one section per version with the groups `Added`, `Changed`, `Deprecated`, `Removed`, `Fixed`, `Security`.
- A change that touches the engine public API or behaviour adds one line under `Unreleased` in the same commit. Commits that only touch the shell, tests, tooling or docs do not.
- Breaking changes are written first in their group and start with `breaking:`.
- A release moves `Unreleased` under the new version and date, bumps `workspace.package.version`, and is tagged `vX.Y.Z`.

## Deprecation

- A public item is deprecated with `#[deprecated(since = "X.Y.Z", note = "use ... instead")]`, and the changelog lists it under `Deprecated`.
- A deprecated item stays for at least one minor release (`0.x`) or one full minor release cycle (`>= 1.0`), and is removed only in a release that the changelog marks `breaking:`.
- The replacement must exist in the release that deprecates the old item.
- Behaviour deprecations (a changed default, a renamed IPC target or settings key) are announced the same way and keep the old spelling working for the same period, logging one warning per run.

## Tooling

- `scripts/check.sh` already builds with `-D warnings`, so use of a deprecated engine item inside the workspace fails the build until it is migrated.
- `cargo semver-checks` runs against the previous tag once the API is declared stable (E7.6); until then the public API list is reviewed by hand at release time.
