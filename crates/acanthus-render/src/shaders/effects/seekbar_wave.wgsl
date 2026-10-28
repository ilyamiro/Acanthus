const TWO_PI: f32 = 6.2831853;

fn ease_quintic(x: f32) -> f32 {
    let t = clamp(x, 0.0, 1.0);
    return t * t * t * (t * (t * 6.0 - 15.0) + 10.0);
}

fn over(dst: vec4<f32>, src: vec4<f32>) -> vec4<f32> {
    return src + dst * (1.0 - src.a);
}

fn capsule(p: vec2<f32>, x0: f32, x1: f32, cy: f32, stroke_width: f32) -> f32 {
    let d = length(p - vec2<f32>(clamp(p.x, x0, x1), cy)) - stroke_width * 0.5;
    return clamp(0.5 - d, 0.0, 1.0);
}

fn wave_layer(p: vec2<f32>, shape: vec4<f32>, motion: vec4<f32>, wave_rgb: vec3<f32>, pad: f32, cy: f32, end_x: f32, stroke_width: f32, amplitude: f32, start_taper: f32, phase: f32) -> vec4<f32> {
    let a = amplitude * shape.x;
    if a < 0.3 || p.x < pad || p.x > end_x {
        return vec4<f32>(0.0);
    }
    let k = TWO_PI / shape.w;
    let n = sin(k * p.x - phase * motion.x + motion.y);
    let swell = sin(k / 6.0 * p.x - phase * motion.x * 0.5 + motion.z);
    let h = 0.5 * (1.0 + n) * (0.8 + 0.2 * swell);
    let grow = ease_quintic((end_x - pad) / (start_taper + shape.z));
    let taper_scale = mix(0.3, 1.0, grow);
    let env = ease_quintic((p.x - pad) / max(start_taper * taper_scale, 1.0)) * ease_quintic((end_x - p.x) / max(shape.z * taper_scale, 1.0));
    let top = cy - stroke_width * 0.5;
    let y_wave = top - h * a * env;
    let coverage = clamp(p.y - y_wave + 0.5, 0.0, 1.0) * clamp(cy - p.y + 0.5, 0.0, 1.0);
    let a1 = shape.y;
    let a0 = select(a1 * 0.55, 1.0, a1 >= 1.0);
    let alpha = mix(a0, a1, clamp((p.y - (top - a)) / a, 0.0, 1.0));
    return vec4<f32>(wave_rgb, 1.0) * alpha * coverage;
}

fn effect(uv: vec2<f32>, size: vec2<f32>, opacity: f32, p0: vec4<f32>, p1: vec4<f32>, p2: vec4<f32>, p3: vec4<f32>) -> vec4<f32> {
    let p = uv * size;
    let wave_rgb = p0.rgb;
    let rgb = vec4<f32>(wave_rgb, 1.0);
    let pad = p1.x;
    let cy = p1.y;
    let end_x = p1.z;
    let stroke_width = p1.w;
    let amplitude = p2.x;
    let track_alpha = p2.y;
    let start_taper = p2.z;
    let phase = p2.w;
    var c = rgb * track_alpha * capsule(p, end_x, size.x - pad, cy, stroke_width);
    c = over(c, wave_layer(p, p3, ext1, wave_rgb, pad, cy, end_x, stroke_width, amplitude, start_taper, phase));
    c = over(c, wave_layer(p, ext0, ext2, wave_rgb, pad, cy, end_x, stroke_width, amplitude, start_taper, phase));
    c = over(c, rgb * capsule(p, pad, end_x, cy, stroke_width));
    return c * opacity;
}
