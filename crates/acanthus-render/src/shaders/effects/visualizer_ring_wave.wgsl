fn level_at(i: i32) -> f32 {
    var v = data_at(u32(i / 4));
    return v[i % 4];
}

fn wrap_index(i: i32, n: i32) -> i32 {
    if i < 0 {
        return i + n;
    }
    if i >= n {
        return i - n;
    }
    return i;
}

fn point_height(i: i32, n: i32, min_length: f32, max_length: f32) -> f32 {
    return max(min_length, level_at(wrap_index(i, n)) * max_length);
}

fn effect(uv: vec2<f32>, size: vec2<f32>, opacity: f32, p0: vec4<f32>, p1: vec4<f32>, p2: vec4<f32>, p3: vec4<f32>) -> vec4<f32> {
    let color_a = p0.a;
    var color_rgb = vec3<f32>(0.0);
    if color_a > 0.0 {
        color_rgb = p0.rgb / color_a;
    }
    let count = p1.x;
    let min_length = p1.y;
    let max_length = p1.z;
    let opacity_base = p1.w;
    let opacity_range = p2.x;
    let ring_radius = p2.y;
    let tint_strength = p2.z;

    let dir = uv * size - size * 0.5;
    let dist = length(dir);
    if dist < 0.5 {
        return vec4<f32>(0.0);
    }

    let n = i32(count);
    let step = 6.28318531 / count;
    var angle = atan2(dir.x, -dir.y);
    if angle < 0.0 {
        angle += 6.28318531;
    }

    let f = angle / step;
    let k = i32(floor(f));
    let u = f - f32(k);
    let h0 = point_height(k, n, min_length, max_length);
    let h1 = point_height(k + 1, n, min_length, max_length);
    let h = mix(h0, h1, u);
    let slope = (h1 - h0) / (step * dist);

    let outer = (dist - ring_radius - h) / sqrt(1.0 + slope * slope);
    let d = max(outer, ring_radius - dist);
    let coverage = clamp(0.5 - d, 0.0, 1.0);
    if coverage <= 0.0 {
        return vec4<f32>(0.0);
    }

    let level = clamp((dist - ring_radius) / max(max_length, 1.0), 0.0, 1.0);
    let rgb = mix(color_rgb, vec3<f32>(1.0), level * tint_strength);
    return vec4<f32>(rgb, 1.0) * color_a * (opacity_base + level * opacity_range) * coverage * opacity;
}
