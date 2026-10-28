const PI: f32 = 3.141592653589793;

fn curtain_layer(accum: vec4<f32>, color: vec4<f32>, i: f32, amp: f32, p: vec2<f32>, size: vec2<f32>, t: f32, b1: f32, b2: f32, phase: f32, reveal: f32, s28: f32) -> vec4<f32> {
    let prog = (reveal - i * 0.07) * 1.55;
    if prog <= 0.0 {
        return accum;
    }
    if prog >= 1.0 {
        return color;
    }
    let smooth_prog = pow(prog, 1.4);
    let current_y = size.y * smooth_prog;
    let wave_amp = s28 * sin(smooth_prog * PI) * amp;
    let offset = i * 0.5;
    let cp1y = current_y + sin(phase + offset) * wave_amp;
    let cp2y = current_y - cos(phase + offset) * wave_amp;
    let u = 1.0 - t;
    let surface_y = u * u * u * current_y + b1 * cp1y + b2 * cp2y + t * t * t * current_y;
    let edge = clamp(0.5 - (p.y - surface_y), 0.0, 1.0);
    return mix(accum, color, edge);
}

fn effect(uv: vec2<f32>, size: vec2<f32>, opacity: f32, p0: vec4<f32>, p1: vec4<f32>, p2: vec4<f32>, p3: vec4<f32>) -> vec4<f32> {
    let reveal = p0.x;
    if reveal <= 0.0 {
        return vec4<f32>(0.0);
    }
    let s28 = p0.y;
    let p = uv * size;
    let t = clamp(p.x / max(size.x, 1.0), 0.0, 1.0);
    let b1 = 3.0 * (1.0 - t) * (1.0 - t) * t;
    let b2 = 3.0 * (1.0 - t) * t * t;
    let phase = reveal * 7.853981633974483;
    var accum = vec4<f32>(0.0);
    accum = curtain_layer(accum, p1, 0.0, 1.5, p, size, t, b1, b2, phase, reveal, s28);
    accum = curtain_layer(accum, p2, 1.0, 1.3, p, size, t, b1, b2, phase, reveal, s28);
    accum = curtain_layer(accum, p3, 2.0, 1.1, p, size, t, b1, b2, phase, reveal, s28);
    accum = curtain_layer(accum, ext0, 3.0, 0.9, p, size, t, b1, b2, phase, reveal, s28);
    accum = curtain_layer(accum, ext1, 4.0, 0.6, p, size, t, b1, b2, phase, reveal, s28);
    return accum * opacity;
}
