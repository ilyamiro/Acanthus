# acanthus-scene

**Parent context:** `../../AGENTS.md`
**Generated:** 2026-10-22 · **Updated:** 2026-10-22

## Purpose
Retained scene graph: node arena, layout, text shaping, display list and input hit testing. No GPU or Wayland code.

## Key Files
| File | Description |
|------|-------------|
| `src/node.rs` | `Node` builder, `Kind {Item, Rect, Text, Image}`, `IntoSignal`, `Gradient`, `TransformOrigin`; per-property `Signal`s (incl. `z`, `scale`, `rotation`, `transform_origin`); `.bind`/`.own`; `paint_order` (stable z sort) |
| `src/scene.rs` | `Scene`: slotmap arena, `frame()` (layout → display list, skipped unless its Tracker is dirty), pointer/scroll dispatch; scene tests |
| `src/layout.rs` | `Anchors` (offsets, sibling refs), `Constraints`, `Layout {Free, Row, Column, Grid, Flow, Stack}`, `Align`; measure (up) / arrange (down) passes |
| `src/display.rs` | `Primitive {Quad, Glyph, Image, LayerBegin, LayerEnd}` list; opacity, `Clip` (device rect + one `ShapeClip`), `Affine2` transform (physical pixels) and z order applied here |
| `src/text.rs` | `Fonts` (shared cosmic-text `FontSystem`), `ShapedText`, `TextParams`, `HAlign`/`VAlign`/`Elide`/`WrapMode` |
| `src/dynamic.rs` | `Scene::repeater` (keyed delegate per `ListSignal` item, `Item {value, index}`), `Scene::loader` (active-driven child), `Host` sync run at the start of `frame()` |
| `src/flick.rs` | `Flickable` (drag, wheel, kinetic flick with Qt constants, rubber band, `BoundsBehavior`, `FlickDirection`); `Scene::flickable` lives in `scene.rs` |
| `src/list_view.rs` | `Scene::list_view`: virtualised ListView over a `ListSignal`, measured extents, `position_view_at_index`, optional add/remove `ListTransitions` |
| `src/keyboard.rs` | `Key`, `KeyEvent`, `Modifiers`, `CursorShape`, `PointerEvent`, process-local `Clipboard` |
| `src/text_input.rs` | `TextInput` editing state, `Scene::text_input` (placeholder, selection, text, caret nodes + host), blink |
| `src/input.rs` | `MouseArea` (hovered, pressed, on_click, on_scroll), `MouseButton`, `ScrollDelta` |

## For AI Agents

### Working In This Directory
- Layout and paint read signals inside `Scene::tracker`; a changed signal dirties only the scenes that read it. Hit testing reads untracked.
- Opacity is per-primitive alpha (Qt semantics); `Node::layer(true)` wraps the subtree in `LayerBegin`/`LayerEnd` for true group opacity, with bounds from descendant rects; `Node::layer_effect(LayerEffect)` adds blur/shadow (`LayerFx` in device px, bounds padded). Transforms: Qt order (origin, rotate, scale), composed down the tree, primitives keep untransformed rects plus a matrix. Clip: `clip(true)` on a `Rect` with radius is rounded; only the innermost rounded or rotated clip is exact, outer ones become bounding boxes. Hit testing inverts transforms and uses rectangular clip like Qt.
- Tests build a `Scene`, call `set_viewport`, `frame()`, then assert `geometry(id)`; copy that style.

### Testing Requirements
`cargo test -p acanthus-scene` (78 tests).

## Dependencies
Internal: acanthus-core, acanthus-anim, acanthus-image. External: cosmic-text, slotmap.
