# acanthus-image

**Parent context:** `../../AGENTS.md`
**Generated:** 2026-10-22 · **Updated:** 2026-10-22

## Purpose
Image and icon loading for the shell. No GPU, scene or Wayland code.

## Key Files
| File | Description |
|------|-------------|
| `src/source.rs` | `ImageSource::parse`: `file://` URLs, `?rev=` queries, percent decoding, `image://icon/name`, bare icon names |
| `src/decode.rs` | `decode_file`, `decode_raster`, `decode_svg` (resvg, no text support): premultiplied RGBA8, optional fit-within target size |
| `src/icons.rs` | `IconLookup`: freedesktop icon theme lookup (index.theme parsing, inheritance, size matching, pixmaps fallback, gtk theme name) |
| `src/fit.rs` | `FillMode {Stretch, PreserveAspectFit, PreserveAspectCrop, Tile}` and `place()` returning the rect and uv |
| `src/store.rs` | `ImageStore`: lazy worker threads, keyed by (source, target size, mtime), ref-counted `ImageHandle`, `poll()`, generation `Signal`, optional waker |

## For AI Agents

### Working In This Directory
- `ImageStore` is `!Send` (UI thread only); workers get plain data. Call `poll()` on the UI thread (the Wayland app does it from a calloop channel fed by the waker).
- Decoded pixels are premultiplied; the renderer relies on that.
- Not done: animated images, SVG re-render at laid-out size, eviction of referenced images, mipmaps.

### Testing Requirements
`cargo test -p acanthus-image` (12 tests, no GPU, no network).

## Dependencies
Internal: acanthus-core. External: image (png, jpeg, webp, gif, bmp), resvg (no default features), tracing.
