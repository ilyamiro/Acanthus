fn effect(uv: vec2<f32>, size: vec2<f32>, opacity: f32, p0: vec4<f32>, p1: vec4<f32>, p2: vec4<f32>, p3: vec4<f32>) -> vec4<f32> {
    let p = (uv - 0.5) * size;
    let dist = length(p);
    let max_dim = max(size.x, size.y);
    let outer_radius = max_dim * select(0.75, p1.y, p1.y > 0.0);
    let inner_radius = outer_radius * select(0.4, p1.x, p1.x > 0.0);
    let max_alpha = select(0.4, p1.z, p1.z > 0.0);
    let t = clamp((dist - inner_radius) / max(outer_radius - inner_radius, 1.0), 0.0, 1.0);
    let a = t * max_alpha * p0.a * opacity;
    return vec4<f32>(p0.rgb * a, a);
}
