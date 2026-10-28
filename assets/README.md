# Test fixtures

Small, fixed inputs read by the engine tests through one helper per crate. Nothing here is shipped to applications.

| Path | Used by | Licence |
|---|---|---|
| `fonts/JetBrainsMono-Regular.ttf` | `acanthus-scene` font loading, `acanthus-runtime` font catalog | SIL Open Font License 1.1, JetBrains |
| `fonts/GoogleSansFlex.ttf` | `acanthus-render` variable-axis glyph test | SIL Open Font License 1.1, Google |
| `themes/*.json` | `acanthus-runtime` preset loading | public colour schemes, values only |
| `languages/*.json` | `acanthus-runtime` i18n loading | written for this repository |
| `matugen/` | `acanthus-runtime` template config parsing and the optional oracle test | written for this repository |
