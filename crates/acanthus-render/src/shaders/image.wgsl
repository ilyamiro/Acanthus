@group(1) @binding(0) var image: texture_2d<f32>;
@group(1) @binding(1) var clamped: sampler;
@group(1) @binding(2) var repeated: sampler;

struct ImageIn {
    @builtin(vertex_index) vi: u32,
    @location(0) rect: vec4<f32>,
    @location(1) uv: vec4<f32>,
    @location(2) clip: vec4<f32>,
    @location(3) params: vec4<f32>,
    @location(4) xform_m: vec4<f32>,
    @location(5) xform_t: vec4<f32>,
    @location(6) shape_rect: vec4<f32>,
    @location(7) shape_m: vec4<f32>,
    @location(8) shape_t: vec4<f32>,
};

struct ImageOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) @interpolate(flat) clip: vec4<f32>,
    @location(2) @interpolate(flat) params: vec4<f32>,
    @location(3) @interpolate(flat) shape_rect: vec4<f32>,
    @location(4) @interpolate(flat) shape_m: vec4<f32>,
    @location(5) @interpolate(flat) shape_t: vec4<f32>,
};

@vertex
fn vs_main(in: ImageIn) -> ImageOut {
    let corner = vec2<f32>(f32(in.vi & 1u), f32(in.vi >> 1u));
    let p = in.rect.xy + corner * in.rect.zw;
    var out: ImageOut;
    out.pos = device_to_ndc(apply_affine(in.xform_m, in.xform_t.xy, p));
    out.uv = in.uv.xy + corner * (in.uv.zw - in.uv.xy);
    out.clip = in.clip;
    out.params = in.params;
    out.shape_rect = in.shape_rect;
    out.shape_m = in.shape_m;
    out.shape_t = in.shape_t;
    return out;
}

@fragment
fn fs_main(in: ImageOut) -> @location(0) vec4<f32> {
    let du = dpdx(in.uv);
    let dv = dpdy(in.uv);
    let clip = clip_coverage(in.pos.xy, in.clip, in.shape_rect, in.shape_m, in.shape_t);
    if clip <= 0.0 {
        discard;
    }
    var texel: vec4<f32>;
    if in.params.y > 0.5 {
        texel = textureSampleGrad(image, repeated, in.uv, du, dv);
    } else {
        texel = textureSampleGrad(image, clamped, in.uv, du, dv);
    }
    return texel * in.params.x * clip;
}
