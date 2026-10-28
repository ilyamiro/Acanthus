fn effect(uv: vec2<f32>, size: vec2<f32>, opacity: f32, p0: vec4<f32>, p1: vec4<f32>, p2: vec4<f32>, p3: vec4<f32>) -> vec4<f32> {
    let p = uv * size;
    let r = min(size.x, size.y);
    var center = vec2<f32>(0.0, 0.0);
    if p0.x < 0.5 {
        center = vec2<f32>(r, r);
    } else if p0.x < 1.5 {
        center = vec2<f32>(0.0, r);
    } else if p0.x < 2.5 {
        center = vec2<f32>(r, 0.0);
    }
    let d = r - length(p - center);
    return p1 * (clamp(0.5 - d, 0.0, 1.0) * opacity);
}
