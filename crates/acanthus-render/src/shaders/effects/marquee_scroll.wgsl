fn effect(uv: vec2<f32>, size: vec2<f32>, opacity: f32, p0: vec4<f32>, p1: vec4<f32>, p2: vec4<f32>, p3: vec4<f32>) -> vec4<f32> {
    let x = (uv.x * size.x + p0.z) / p0.x;
    return sample0(vec2<f32>(x, uv.y)) * opacity;
}
