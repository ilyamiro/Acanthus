struct QuadIn {
    @builtin(vertex_index) vi: u32,
    @location(0) rect: vec4<f32>,
    @location(1) start: vec4<f32>,
    @location(2) end: vec4<f32>,
    @location(3) border_color: vec4<f32>,
    @location(4) params: vec4<f32>,
    @location(5) clip: vec4<f32>,
    @location(6) xform_m: vec4<f32>,
    @location(7) xform_t: vec4<f32>,
    @location(8) shape_rect: vec4<f32>,
    @location(9) shape_m: vec4<f32>,
    @location(10) shape_t: vec4<f32>,
    @location(11) radii: vec4<f32>,
};

struct QuadOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) pixel: vec2<f32>,
    @location(1) @interpolate(flat) rect: vec4<f32>,
    @location(2) @interpolate(flat) start: vec4<f32>,
    @location(3) @interpolate(flat) end: vec4<f32>,
    @location(4) @interpolate(flat) border_color: vec4<f32>,
    @location(5) @interpolate(flat) params: vec4<f32>,
    @location(6) @interpolate(flat) clip: vec4<f32>,
    @location(7) @interpolate(flat) shape_rect: vec4<f32>,
    @location(8) @interpolate(flat) shape_m: vec4<f32>,
    @location(9) @interpolate(flat) shape_t: vec4<f32>,
    @location(10) @interpolate(flat) radii: vec4<f32>,
};

@vertex
fn vs_main(in: QuadIn) -> QuadOut {
    let corner = vec2<f32>(f32(in.vi & 1u), f32(in.vi >> 1u));
    let pad = vec2<f32>(1.0 / max(in.xform_t.z, 0.0001));
    let p = in.rect.xy - pad + corner * (in.rect.zw + 2.0 * pad);
    var out: QuadOut;
    out.pos = device_to_ndc(apply_affine(in.xform_m, in.xform_t.xy, p));
    out.pixel = p;
    out.rect = in.rect;
    out.start = in.start;
    out.end = in.end;
    out.border_color = in.border_color;
    out.params = vec4<f32>(in.params.xyz, in.xform_t.z);
    out.clip = in.clip;
    out.shape_rect = in.shape_rect;
    out.shape_m = in.shape_m;
    out.shape_t = in.shape_t;
    out.radii = in.radii;
    return out;
}

@fragment
fn fs_main(in: QuadOut) -> @location(0) vec4<f32> {
    let clip = clip_coverage(in.pos.xy, in.clip, in.shape_rect, in.shape_m, in.shape_t);
    if clip <= 0.0 {
        discard;
    }
    let half = in.rect.zw * 0.5;
    let local = in.pixel - in.rect.xy - half;
    let border = in.params.y;
    let mode = in.params.z;
    let scale = in.params.w;

    let d = sd_round_box4(local, half, in.radii) * scale;
    let coverage = clamp(0.5 - d, 0.0, 1.0) * clip;
    if coverage <= 0.0 {
        discard;
    }

    var t = 0.0;
    if mode > 1.5 {
        t = clamp((in.pixel.x - in.rect.x) / max(in.rect.z, 1.0), 0.0, 1.0);
    } else if mode > 0.5 {
        t = clamp((in.pixel.y - in.rect.y) / max(in.rect.w, 1.0), 0.0, 1.0);
    }
    var color = mix(in.start, in.end, t);

    if border > 0.0 {
        let inner_half = max(half - vec2<f32>(border), vec2<f32>(0.0));
        let inner_d = sd_round_box4(local, inner_half, max(in.radii - vec4<f32>(border), vec4<f32>(0.0))) * scale;
        let inner = clamp(0.5 - inner_d, 0.0, 1.0);
        color = mix(in.border_color, color, inner);
    }
    return color * coverage;
}
