struct Globals {
    viewport: vec2<f32>,
    origin: vec2<f32>,
};

@group(0) @binding(0) var<uniform> globals: Globals;

fn apply_affine(m: vec4<f32>, t: vec2<f32>, p: vec2<f32>) -> vec2<f32> {
    return vec2<f32>(m.x * p.x + m.z * p.y, m.y * p.x + m.w * p.y) + t;
}

fn device_to_ndc(p: vec2<f32>) -> vec4<f32> {
    return vec4<f32>((p.x - globals.origin.x) / globals.viewport.x * 2.0 - 1.0, 1.0 - (p.y - globals.origin.y) / globals.viewport.y * 2.0, 0.0, 1.0);
}

fn sd_round_box(p: vec2<f32>, half: vec2<f32>, r: f32) -> f32 {
    let q = abs(p) - half + vec2<f32>(r);
    return length(max(q, vec2<f32>(0.0))) + min(max(q.x, q.y), 0.0) - r;
}

fn sd_round_box4(p: vec2<f32>, half: vec2<f32>, r: vec4<f32>) -> f32 {
    let top = select(r.x, r.y, p.x > 0.0);
    let bottom = select(r.w, r.z, p.x > 0.0);
    let rr = select(top, bottom, p.y > 0.0);
    let q = abs(p) - half + vec2<f32>(rr);
    return length(max(q, vec2<f32>(0.0))) + min(max(q.x, q.y), 0.0) - rr;
}

fn clip_coverage(window: vec2<f32>, clip: vec4<f32>, shape_rect: vec4<f32>, shape_m: vec4<f32>, shape_t: vec4<f32>) -> f32 {
    let frag = window + globals.origin;
    if frag.x < clip.x || frag.y < clip.y || frag.x >= clip.x + clip.z || frag.y >= clip.y + clip.w {
        return 0.0;
    }
    if shape_t.z < 0.0 {
        return 1.0;
    }
    let local = apply_affine(shape_m, shape_t.xy, frag);
    let half = shape_rect.zw * 0.5;
    let d = sd_round_box(local - shape_rect.xy - half, half, shape_t.z) * shape_t.w;
    return clamp(0.5 - d, 0.0, 1.0);
}
