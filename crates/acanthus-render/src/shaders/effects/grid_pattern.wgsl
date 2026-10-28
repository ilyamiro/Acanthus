fn effect(uv: vec2<f32>, size: vec2<f32>, opacity: f32, p0: vec4<f32>, p1: vec4<f32>, p2: vec4<f32>, p3: vec4<f32>) -> vec4<f32> {
    let p = uv * size;
    let step_size = select(16.0, p0.x, p0.x > 0.0);
    let line_width = select(1.0, p0.y, p0.y > 0.0);
    let grid_mod = glsl_mod(p, step_size);
    let dx = min(grid_mod.x, step_size - grid_mod.x);
    let dy = min(grid_mod.y, step_size - grid_mod.y);
    let d = min(dx, dy) - line_width * 0.5;
    let line_alpha = clamp(0.5 - d, 0.0, 1.0) * p1.a * opacity;
    return vec4<f32>(p1.rgb * line_alpha, line_alpha);
}
