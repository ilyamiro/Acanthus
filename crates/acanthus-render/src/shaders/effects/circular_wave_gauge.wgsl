const PI: f32 = 3.141592653589793;

fn effect(uv: vec2<f32>, size: vec2<f32>, opacity: f32, p0: vec4<f32>, p1: vec4<f32>, p2: vec4<f32>, p3: vec4<f32>) -> vec4<f32> {
    let fill_ratio = p0.x;
    let stroke_width = p0.y;
    let use_sine = p0.z;
    let p = uv * size;
    let center = size * 0.5;
    let d = p - center;
    let dist = length(d);
    let stroke_w = select(2.2, stroke_width, stroke_width > 0.0);
    let amp = select(0.9, p2.x, p2.x > 0.0);
    let radius = select(min(center.x, center.y) - amp - stroke_w * 0.5 - 0.4, p2.y, p2.y > 0.0);
    if radius <= 0.0 {
        return vec4<f32>(0.0);
    }
    let d_track = abs(dist - radius) - stroke_w * 0.5;
    let track_alpha = clamp(0.5 - d_track, 0.0, 1.0) * 0.22;
    let track_col = vec4<f32>(p1.rgb, 1.0) * track_alpha;
    if fill_ratio <= 0.001 {
        let ta = track_alpha * opacity;
        return vec4<f32>(track_col.rgb * ta, ta);
    }
    let angle = atan2(d.y, d.x);
    let norm_angle = glsl_mod1(angle + PI * 0.5, 2.0 * PI);
    let sweep_angle = 2.0 * PI * clamp(fill_ratio, 0.0, 1.0);
    var d_prog = 1e6;
    var r_target = radius;
    if use_sine > 0.5 {
        let total_p = 2.0 * PI * radius;
        let cycles = max(5.0, floor(total_p / 8.5 + 0.5));
        let freq = (2.0 * PI * cycles) / total_p;
        r_target += amp * sin(freq * (norm_angle * radius));
    }
    if fill_ratio >= 0.999 {
        d_prog = abs(dist - r_target) - stroke_w * 0.5;
    } else {
        if norm_angle <= sweep_angle {
            d_prog = abs(dist - r_target) - stroke_w * 0.5;
        }
        let start_cap = center + vec2<f32>(0.0, -radius);
        let end_a = -PI * 0.5 + sweep_angle;
        let end_cap = center + radius * vec2<f32>(cos(end_a), sin(end_a));
        let d_cap0 = length(p - start_cap) - stroke_w * 0.5;
        let d_cap1 = length(p - end_cap) - stroke_w * 0.5;
        d_prog = min(d_prog, min(d_cap0, d_cap1));
    }
    let prog_alpha = clamp(0.5 - d_prog, 0.0, 1.0);
    let prog_col = vec4<f32>(p1.rgb, 1.0) * prog_alpha;
    let res = prog_col + track_col * (1.0 - prog_alpha);
    let final_a = res.a * opacity;
    return vec4<f32>(res.rgb * final_a, final_a);
}
