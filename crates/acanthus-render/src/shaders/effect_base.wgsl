@group(1) @binding(0) var src0: texture_2d<f32>;
@group(1) @binding(1) var src0_sampler: sampler;
@group(2) @binding(0) var src1: texture_2d<f32>;
@group(2) @binding(1) var src1_sampler: sampler;

@group(3) @binding(0) var<storage, read> effect_data: array<vec4<f32>>;

var<private> data_base: u32;
var<private> frag_coord: vec2<f32>;
var<private> ext0: vec4<f32>;
var<private> ext1: vec4<f32>;
var<private> ext2: vec4<f32>;
var<private> ext3: vec4<f32>;

fn data_at(i: u32) -> vec4<f32> {
    return effect_data[data_base + i];
}

fn sample0(uv: vec2<f32>) -> vec4<f32> {
    return textureSampleLevel(src0, src0_sampler, uv, 0.0);
}

fn sample1(uv: vec2<f32>) -> vec4<f32> {
    return textureSampleLevel(src1, src1_sampler, uv, 0.0);
}

struct EffectIn {
    @builtin(vertex_index) vi: u32,
    @location(0) rect: vec4<f32>,
    @location(1) clip: vec4<f32>,
    @location(2) info: vec4<f32>,
    @location(3) p0: vec4<f32>,
    @location(4) p1: vec4<f32>,
    @location(5) p2: vec4<f32>,
    @location(6) p3: vec4<f32>,
    @location(7) p4: vec4<f32>,
    @location(8) p5: vec4<f32>,
    @location(9) p6: vec4<f32>,
    @location(10) p7: vec4<f32>,
    @location(11) xform_m: vec4<f32>,
    @location(12) xform_t: vec4<f32>,
    @location(13) shape_rect: vec4<f32>,
    @location(14) shape_m: vec4<f32>,
    @location(15) shape_t: vec4<f32>,
};

struct EffectOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) @interpolate(flat) clip: vec4<f32>,
    @location(2) @interpolate(flat) info: vec4<f32>,
    @location(3) @interpolate(flat) p0: vec4<f32>,
    @location(4) @interpolate(flat) p1: vec4<f32>,
    @location(5) @interpolate(flat) p2: vec4<f32>,
    @location(6) @interpolate(flat) p3: vec4<f32>,
    @location(7) @interpolate(flat) p4: vec4<f32>,
    @location(8) @interpolate(flat) p5: vec4<f32>,
    @location(9) @interpolate(flat) p6: vec4<f32>,
    @location(10) @interpolate(flat) p7: vec4<f32>,
    @location(11) @interpolate(flat) shape_rect: vec4<f32>,
    @location(12) @interpolate(flat) shape_m: vec4<f32>,
    @location(13) @interpolate(flat) shape_t: vec4<f32>,
};

fn glsl_mod(x: vec2<f32>, y: f32) -> vec2<f32> {
    return x - y * floor(x / y);
}

fn glsl_mod1(x: f32, y: f32) -> f32 {
    return x - y * floor(x / y);
}

@vertex
fn vs_main(in: EffectIn) -> EffectOut {
    let corner = vec2<f32>(f32(in.vi & 1u), f32(in.vi >> 1u));
    let p = in.rect.xy + corner * in.rect.zw;
    var out: EffectOut;
    out.pos = device_to_ndc(apply_affine(in.xform_m, in.xform_t.xy, p));
    out.uv = corner;
    out.clip = in.clip;
    out.info = in.info;
    out.p0 = in.p0;
    out.p1 = in.p1;
    out.p2 = in.p2;
    out.p3 = in.p3;
    out.p4 = in.p4;
    out.p5 = in.p5;
    out.p6 = in.p6;
    out.p7 = in.p7;
    out.shape_rect = in.shape_rect;
    out.shape_m = in.shape_m;
    out.shape_t = in.shape_t;
    return out;
}

@fragment
fn fs_main(in: EffectOut) -> @location(0) vec4<f32> {
    let clip = clip_coverage(in.pos.xy, in.clip, in.shape_rect, in.shape_m, in.shape_t);
    if clip <= 0.0 {
        discard;
    }
    let size = in.info.yz;
    data_base = u32(in.info.w);
    frag_coord = in.pos.xy;
    ext0 = in.p4;
    ext1 = in.p5;
    ext2 = in.p6;
    ext3 = in.p7;
    return effect(in.uv, size, in.info.x, in.p0, in.p1, in.p2, in.p3) * clip;
}
