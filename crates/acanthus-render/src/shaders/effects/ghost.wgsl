fn disc(p: vec2<f32>, c: vec2<f32>, r: f32) -> f32 {
    return clamp(0.5 - (length(p - c) - r), 0.0, 1.0);
}

fn effect(uv: vec2<f32>, size: vec2<f32>, opacity: f32, p0: vec4<f32>, p1: vec4<f32>, p2: vec4<f32>, p3: vec4<f32>) -> vec4<f32> {
    let p = uv * size;
    let w = size.x - 2.0;
    let h = size.y - 2.0;
    let r = w * 0.5;
    let bottom = 1.0 + h;
    let wave_h = h * 0.14;
    let centre = vec2<f32>(1.0 + r, 1.0 + r);
    var d = 0.0;
    if p.y < centre.y {
        d = length(p - centre) - r;
    } else {
        let dx = max(p.x - (1.0 + w), 1.0 - p.x);
        let period = w / 3.0;
        let u = fract((p.x - 1.0) / period);
        let yb = bottom - 2.0 * wave_h * u * (1.0 - u);
        let slope = -2.0 * wave_h * (1.0 - 2.0 * u) / period;
        d = max(dx, (p.y - yb) / sqrt(1.0 + slope * slope));
    }
    let body_a = clamp(0.5 - d, 0.0, 1.0);
    let eye_r = w * 0.16;
    let eye_y = 1.0 + r * 0.95;
    let e1 = vec2<f32>(1.0 + w * 0.32, eye_y);
    let e2 = vec2<f32>(1.0 + w * 0.68, eye_y);
    let eyes_a = max(disc(p, e1, eye_r), disc(p, e2, eye_r));
    let shift = p0.xy * eye_r;
    let pupil_r = eye_r * 0.55;
    let pupils_a = max(disc(p, e1 + shift, pupil_r), disc(p, e2 + shift, pupil_r));
    let base = vec4<f32>(p1.rgb * p1.a, p1.a) * body_a;
    let white = vec4<f32>(eyes_a);
    let pupil = vec4<f32>(p2.rgb * p2.a, p2.a) * pupils_a;
    let with_eyes = white + base * (1.0 - white.a);
    return (pupil + with_eyes * (1.0 - pupil.a)) * opacity;
}
