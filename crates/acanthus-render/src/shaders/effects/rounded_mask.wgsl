fn effect(uv: vec2<f32>, size: vec2<f32>, opacity: f32, p0: vec4<f32>, p1: vec4<f32>, p2: vec4<f32>, p3: vec4<f32>) -> vec4<f32> {
    let p = uv * size - size * 0.5;
    let h = size * 0.5;
    let left = select(p0.w, p0.x, p.y < 0.0);
    let right = select(p0.z, p0.y, p.y < 0.0);
    let r = min(select(right, left, p.x < 0.0), min(h.x, h.y));
    let q = abs(p) - h + r;
    let d = min(max(q.x, q.y), 0.0) + length(max(q, vec2<f32>(0.0))) - r;
    let c = clamp(0.5 - d, 0.0, 1.0);
    let fill = p1.x;
    let cover = c * (1.0 - fill) / max(1.0 - c * fill, 0.0001);
    return sample0(uv) * (cover * opacity);
}
