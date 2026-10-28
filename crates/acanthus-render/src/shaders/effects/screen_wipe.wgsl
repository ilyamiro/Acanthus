const PI: f32 = 3.141592653589793;

fn screen_layer(accum: vec4<f32>, color: vec4<f32>, i: f32, amp: f32, p: vec2<f32>, size: vec2<f32>, t: f32, b1: f32, b2: f32, base_amp: f32, reveal: f32, phase: f32) -> vec4<f32> {
    let prog = clamp((reveal - i * 0.05) * 2.5, 0.0, 1.0);
    if prog <= 0.0 {
        return accum;
    }
    if prog >= 1.0 {
        return color;
    }
    let current_x = size.x * prog;
    let wave_amp = base_amp * sin(prog * PI) * amp;
    let offset = i * 0.5;
    let cp1x = current_x + sin(phase + offset) * wave_amp;
    let cp2x = current_x - cos(phase + offset) * wave_amp;
    let u = 1.0 - t;
    let surface_x = u * u * u * current_x + b1 * cp1x + b2 * cp2x + t * t * t * current_x;
    let edge = clamp(0.5 - (p.x - surface_x), 0.0, 1.0);
    return mix(accum, color, edge);
}

fn effect(uv: vec2<f32>, size: vec2<f32>, opacity: f32, p0: vec4<f32>, p1: vec4<f32>, p2: vec4<f32>, p3: vec4<f32>) -> vec4<f32> {
    let reveal = p0.x;
    if reveal <= 0.0 {
        return vec4<f32>(0.0);
    }
    let phase = p0.y;
    let p = uv * size;
    let t = clamp(p.y / max(size.y, 1.0), 0.0, 1.0);
    let b1 = 3.0 * (1.0 - t) * (1.0 - t) * t;
    let b2 = 3.0 * (1.0 - t) * t * t;
    let base_amp = select(45.0, p0.z, p0.z > 0.0);
    var accum = vec4<f32>(0.0);
    accum = screen_layer(accum, p1, 0.0, 1.8, p, size, t, b1, b2, base_amp, reveal, phase);
    accum = screen_layer(accum, p2, 1.0, 1.5, p, size, t, b1, b2, base_amp, reveal, phase);
    accum = screen_layer(accum, p3, 2.0, 1.2, p, size, t, b1, b2, base_amp, reveal, phase);
    accum = screen_layer(accum, ext0, 3.0, 0.9, p, size, t, b1, b2, base_amp, reveal, phase);
    accum = screen_layer(accum, ext1, 4.0, 0.6, p, size, t, b1, b2, base_amp, reveal, phase);
    return accum * opacity;
}
