const TAU: f32 = 6.283185307179586;
const STEPS: i32 = 32;

fn arc_length(radii: vec2<f32>, t: f32) -> f32 {
    let h = t / f32(STEPS);
    var acc = 0.0;
    for (var i = 0; i < STEPS; i = i + 1) {
        let m = (f32(i) + 0.5) * h;
        let sx = radii.x * sin(m);
        let cy = radii.y * cos(m);
        acc = acc + sqrt(sx * sx + cy * cy);
    }
    return acc * h;
}

fn effect(uv: vec2<f32>, size: vec2<f32>, opacity: f32, p0: vec4<f32>, p1: vec4<f32>, p2: vec4<f32>, p3: vec4<f32>) -> vec4<f32> {
    let radii = p0.xy;
    let dash = p0.zw;
    let line_width = p1.x;
    let d = uv * size - size * 0.5;
    let q = d / radii;
    let ql = length(q);
    if ql < 1e-4 {
        return vec4<f32>(0.0);
    }
    let grad = length(d / (radii * radii)) / ql;
    let perp = (ql - 1.0) / grad;
    let half_line = line_width * 0.5;
    if abs(perp) > half_line + dash.x + 1.0 {
        return vec4<f32>(0.0);
    }
    var t = atan2(q.y, q.x);
    if t < 0.0 {
        t = t + TAU;
    }
    let period = dash.x + dash.y;
    let s = arc_length(radii, t);
    let uc = glsl_mod1(s - dash.x * 0.5 + period * 0.5, period) - period * 0.5;
    let along = max(abs(uc) - dash.x * 0.5, 0.0);
    let dist = length(vec2<f32>(along, perp)) - half_line;
    let alpha = clamp(0.5 - dist, 0.0, 1.0) * p2.a * opacity;
    return vec4<f32>(p2.rgb * alpha, alpha);
}
