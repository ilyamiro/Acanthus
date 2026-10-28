fn effect(uv: vec2<f32>, size: vec2<f32>, opacity: f32, p0: vec4<f32>, p1: vec4<f32>, p2: vec4<f32>, p3: vec4<f32>) -> vec4<f32> {
    let radius = p0.x;
    let fill_level = p0.y;
    let wave_amp = p0.z;
    let phase = p0.w;
    let vertical = p1.x;
    if fill_level <= 0.0001 {
        return vec4<f32>(0.0);
    }
    let p = uv * size;
    let half_size = size * 0.5;
    let clamped_r = clamp(radius, 0.0, min(half_size.x, half_size.y));
    let d_box = sd_round_box(p - half_size, half_size, clamped_r);
    let box_alpha = clamp(0.5 - d_box, 0.0, 1.0);
    var fluid_alpha = 0.0;
    let global_alpha = select(1.0, p1.y, p1.y > 0.0);
    if vertical > 0.5 {
        if fill_level >= 0.999 {
            fluid_alpha = 1.0;
        } else {
            let fill_y = size.y * (1.0 - fill_level);
            let t = clamp(p.x / max(size.x, 1.0), 0.0, 1.0);
            let b1 = 3.0 * (1.0 - t) * (1.0 - t) * t;
            let b2 = 3.0 * (1.0 - t) * t * t;
            let wave_offset = wave_amp * (-b1 * cos(phase) + b2 * sin(phase));
            fluid_alpha = clamp(0.5 - ((fill_y + wave_offset) - p.y), 0.0, 1.0);
        }
    } else {
        if fill_level >= 0.999 {
            fluid_alpha = 1.0;
        } else {
            let fill_x = size.x * fill_level;
            let t = clamp(p.y / max(size.y, 1.0), 0.0, 1.0);
            let b1 = 3.0 * (1.0 - t) * (1.0 - t) * t;
            let b2 = 3.0 * (1.0 - t) * t * t;
            let wave_offset = wave_amp * (b2 * sin(phase) - b1 * cos(phase));
            fluid_alpha = clamp(0.5 - (p.x - (fill_x + wave_offset)), 0.0, 1.0);
        }
    }
    var col: vec4<f32>;
    if vertical > 0.5 {
        col = mix(p2, p3, clamp(p.y / max(size.y, 1.0), 0.0, 1.0));
    } else {
        col = mix(p2, p3, clamp(p.x / max(size.x, 1.0), 0.0, 1.0));
    }
    let a = box_alpha * fluid_alpha * global_alpha * col.a * opacity;
    return vec4<f32>(col.rgb * a, a);
}
