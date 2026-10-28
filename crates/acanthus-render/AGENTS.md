# acanthus-render

**Parent context:** `../../AGENTS.md`
**Generated:** 2026-10-22 · **Updated:** 2026-10-22

## Purpose
wgpu renderer that draws a acanthus-scene display list: instanced SDF rounded rects and a glyph pipeline and an image pipeline.

## Key Files
| File | Description |
|------|-------------|
| `src/gpu.rs` | `Gpu` (instance, LowPower adapter, device, queue), `surface_config`, `GpuError` |
| `src/renderer.rs` | `Renderer`: quad + glyph pipelines, paint-order batching, premultiplied gamma-space blending |
| `src/images.rs` | `GpuImages`: one premultiplied `Rgba8Unorm` texture + bind group per `ImageId`, clamp and repeat samplers, garbage collection against the `ImageStore` |
| `src/atlas.rs` | Glyph atlas: 1024² shelf pages (R8 masks, RGBA colour glyphs), LRU page eviction, shared through `GpuContext` |
| `src/shaders/` | `blur.wgsl` is standalone (no common.wgsl); other WGSL shaders (no comments allowed); `common.wgsl` (globals, `apply_affine`, `sd_round_box`, `clip_coverage`) is prepended to each shader by `renderer::shader` |
| `tests/headless.rs` | Offscreen render + pixel readback tests; skip without an adapter |

## For AI Agents

### Working In This Directory
- wgpu 30 API quirks: `Queue::present`, `CurrentSurfaceTexture`, `Option` in buffer/bind-group layout slices, `immediate_size`, `multiview_mask`. Check the crate source.
- Blending is in sRGB space on a `*Unorm` target on purpose (matches Qt's look).
- Every instance ends with a `Placement` (transform matrix + translation + uniform scale, shape clip rect + inverse matrix + radius + scale; radius `-1` means no shape clip). Identity transforms must stay pixel-exact with the untransformed path (glyphs use `textureLoad` unless transformed).
- Layers: `prepare` splits the display list into `Segment`s (one per layer plus the root); each renders in its own pass, last segment first, into pooled textures; `Globals` carries the segment origin and lives in a dynamic-offset ring. Layer composites reuse the image pipeline.
- Effects: a segment with `LayerFx` gets extra blur passes (`shaders/blur.wgsl`, params in a second dynamic-offset ring) after its content pass; `Pass::Shadow` and `Pass::Layer` composite the shadow and result textures. Textures come from `take_texture` (reuse by size, unused ones dropped each frame).
- Mask and colour adjustments run through `shaders/adjust.wgsl` (mask segment texture bound as group 2). Layer textures are cached by segment hash (`store_cache`/`plan_textures`); cached segments skip their passes.
- ShaderEffect recipe (sourceless): add a variant to `acanthus_scene::ShaderEffect` (+ `ALL`) and a `ShaderParams` constructor packing the QML properties into up to 8 vec4 slots via `ShaderParams::pack` (slots 0 to 3 are `p0` to `p3` of `effect`, slots 4 to 7 are the private globals `ext0` to `ext3` set by `effect_base.wgsl`; the instance uses all 16 vertex attribute locations) (colours via `premultiplied()`); write `shaders/effects/<name>.wgsl` defining `fn effect(uv, size, opacity, p0, p1, p2, p3) -> vec4<f32>` (premultiplied output, `size` = logical item size, `glsl_mod`/`glsl_mod1` replace GLSL `mod`); register it in `effect_source`; add a headless pixel test. `effect_base.wgsl` supplies vertex stage, clip and transform handling.
- Textured effects: `Node::shader(..).shader_sources([a, b])` names up to two source nodes (declare them with `mask_source()`). `DisplayCx::emit_source` wraps each in a `LayerBegin { source: Some(slot) }` segment right before the shader primitive; the renderer binds the source results as groups 1 and 2 (`image` layout, a 1x1 blank texture when absent) and `effect_base.wgsl` exposes `sample0(uv)` / `sample1(uv)` (level 0). The consuming segment's hash includes the source hashes. No mipmaps yet.
- Data buffer: `Node::shader_data(ShaderData)` (vec4s, `ShaderData::from_floats` packs 4 floats per vec4) is appended to one per-frame storage buffer (`DataBuffer`, group 3); the instance carries the offset in `info.w` and WGSL reads `data_at(i)`. Use it for arrays beyond the 8 param slots.
- `GpuContext` (from `Gpu::context(format, &ImageStore)`) is shared by every surface of the same format and store; `Renderer::with_context` builds the per-surface part. Atlas page eviction bumps `generation()`, which drops the layer cache.

### Testing Requirements
`cargo test -p acanthus-render` (1 unit + 40 headless GPU).

## Dependencies
Internal: acanthus-core, acanthus-scene (re-exports acanthus-image types). External: wgpu, cosmic-text, bytemuck.
