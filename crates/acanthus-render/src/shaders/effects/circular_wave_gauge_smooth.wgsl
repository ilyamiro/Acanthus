const PI: f32 = 3.141592653589793;

fn curve_point(t: f32, center: vec2<f32>, radius: f32, amp: f32, cycles: f32, use_sine: f32) -> vec2<f32> {
    let r = radius + select(0.0, amp * sin(t * cycles), use_sine > 0.5);
    let a = t - PI * 0.5;
    return center + r * vec2<f32>(cos(a), sin(a));
}

fn fold_t(t: f32, t_max: f32) -> f32 {
    return select(t, t - 2.0 * PI, t_max < 100.0 && t > 2.0 * PI);
}

fn arc_distance(p: vec2<f32>, center: vec2<f32>, radius: f32, amp: f32, cycles: f32, use_sine: f32, t0: f32, w: f32, t_min: f32, t_max: f32) -> f32 {
    var best = 1e6;
    var best_t = t0;
    for (var i = 0; i <= 16; i++) {
        let t = t0 - w + 2.0 * w * f32(i) / 16.0;
        let d = length(p - curve_point(clamp(fold_t(t, t_max), t_min, t_max), center, radius, amp, cycles, use_sine));
        if d < best {
            best = d;
            best_t = t;
        }
    }
    var h = 2.0 * w / 16.0;
    for (var k = 0; k < 4; k++) {
        for (var s = -1; s <= 1; s += 2) {
            let t = best_t + f32(s) * h * 0.5;
            let d = length(p - curve_point(clamp(fold_t(t, t_max), t_min, t_max), center, radius, amp, cycles, use_sine));
            if d < best {
                best = d;
                best_t = t;
            }
        }
        h *= 0.5;
    }
    return best;
}

fn effect(uv: vec2<f32>, size: vec2<f32>, opacity: f32, p0: vec4<f32>, p1: vec4<f32>, p2: vec4<f32>, p3: vec4<f32>) -> vec4<f32> {
    let fill_ratio = p0.x;
    let stroke_width = p0.y;
    let use_sine = p0.z;
    let p = uv * size;
    let center = size * 0.5;
    let d = p - center;
    let dist = length(d);
    let outer = min(center.x, center.y) - 1.0;
    if outer <= 0.0 {
        return vec4<f32>(0.0);
    }
    let stroke_w = min(select(2.2, stroke_width, stroke_width > 0.0), outer * 0.4);
    let half_w = stroke_w * 0.5;
    var amp = 0.0;
    if use_sine > 0.5 {
        amp = select(stroke_w * 0.35, p2.x, p2.x > 0.0);
    }
    amp = min(amp, outer * 0.05);
    let radius = select(outer - amp - half_w, p2.y, p2.y > 0.0);
    if radius <= 0.0 {
        return vec4<f32>(0.0);
    }
    let cycles = select(12.0, p2.z, p2.z > 0.0);
    amp = min(amp, radius * 0.05);
    let reach = half_w + amp + 1.5;
    if dist < radius - reach || dist > radius + reach {
        return vec4<f32>(0.0);
    }
    var theta = atan2(d.y, d.x) + PI * 0.5;
    if theta < 0.0 {
        theta += 2.0 * PI;
    }
    let w = select(PI, asin(min(1.0, reach / dist)), dist > reach);
    let d_track = arc_distance(p, center, radius, amp, cycles, use_sine, theta, w, -1e3, 1e3) - half_w;
    if d_track >= 0.5 {
        return vec4<f32>(0.0);
    }
    let track_alpha = clamp(0.5 - d_track, 0.0, 1.0) * p3.a;
    let fill = clamp(fill_ratio, 0.0, 1.0);
    let sweep = 2.0 * PI * fill;
    var d_prog = 1e6;
    if fill >= 0.999 {
        d_prog = d_track;
    } else if fill > 0.001 {
        d_prog = arc_distance(p, center, radius, amp, cycles, use_sine, theta, w, 0.0, sweep) - half_w;
    }
    let prog_alpha = clamp(0.5 - d_prog, 0.0, 1.0);
    let a = prog_alpha + track_alpha * (1.0 - prog_alpha);
    let rgb = p1.rgb * prog_alpha + p3.rgb * track_alpha * (1.0 - prog_alpha);
    return vec4<f32>(rgb * opacity, a * opacity);
}
