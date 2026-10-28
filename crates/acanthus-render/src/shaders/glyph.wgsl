@group(1) @binding(0) var atlas: texture_2d<f32>;

struct GlyphIn {
    @builtin(vertex_index) vi: u32,
    @location(0) rect: vec4<f32>,
    @location(1) uv: vec4<f32>,
    @location(2) color: vec4<f32>,
    @location(3) clip: vec4<f32>,
    @location(4) params: vec4<f32>,
    @location(5) xform_m: vec4<f32>,
    @location(6) xform_t: vec4<f32>,
    @location(7) shape_rect: vec4<f32>,
    @location(8) shape_m: vec4<f32>,
    @location(9) shape_t: vec4<f32>,
};

struct GlyphOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) @interpolate(flat) color: vec4<f32>,
    @location(2) @interpolate(flat) clip: vec4<f32>,
    @location(3) @interpolate(flat) params: vec4<f32>,
    @location(4) @interpolate(flat) region: vec4<f32>,
    @location(5) @interpolate(flat) shape_rect: vec4<f32>,
    @location(6) @interpolate(flat) shape_m: vec4<f32>,
    @location(7) @interpolate(flat) shape_t: vec4<f32>,
};

@vertex
fn vs_main(in: GlyphIn) -> GlyphOut {
    let corner = vec2<f32>(f32(in.vi & 1u), f32(in.vi >> 1u));
    let p = in.rect.xy + corner * in.rect.zw;
    var out: GlyphOut;
    out.pos = device_to_ndc(apply_affine(in.xform_m, in.xform_t.xy, p));
    out.uv = in.uv.xy + corner * in.uv.zw;
    out.color = in.color;
    out.clip = in.clip;
    out.params = in.params;
    out.region = in.uv;
    out.shape_rect = in.shape_rect;
    out.shape_m = in.shape_m;
    out.shape_t = in.shape_t;
    return out;
}

fn texel_at(region: vec4<f32>, p: vec2<f32>) -> vec4<f32> {
    let lo = region.xy;
    let hi = region.xy + region.zw - vec2<f32>(1.0);
    if p.x < lo.x || p.y < lo.y || p.x > hi.x || p.y > hi.y {
        return vec4<f32>(0.0);
    }
    return textureLoad(atlas, vec2<i32>(p), 0);
}

fn sample_bilinear(region: vec4<f32>, uv: vec2<f32>) -> vec4<f32> {
    let p = uv - vec2<f32>(0.5);
    let base = floor(p);
    let f = p - base;
    let a = mix(texel_at(region, base), texel_at(region, base + vec2<f32>(1.0, 0.0)), f.x);
    let b = mix(texel_at(region, base + vec2<f32>(0.0, 1.0)), texel_at(region, base + vec2<f32>(1.0, 1.0)), f.x);
    return mix(a, b, f.y);
}

@fragment
fn fs_main(in: GlyphOut) -> @location(0) vec4<f32> {
    let clip = clip_coverage(in.pos.xy, in.clip, in.shape_rect, in.shape_m, in.shape_t);
    if clip <= 0.0 {
        discard;
    }
    var texel: vec4<f32>;
    if in.params.y > 0.5 {
        texel = sample_bilinear(in.region, in.uv);
    } else {
        texel = textureLoad(atlas, vec2<i32>(floor(in.uv)), 0);
    }
    if in.params.x > 0.5 {
        return vec4<f32>(texel.rgb * texel.a, texel.a) * in.color.a * clip;
    }
    return in.color * texel.r * clip;
}
