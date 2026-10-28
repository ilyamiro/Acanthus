fn level_at(i: i32) -> f32 {
    var v = data_at(u32(i / 4));
    return v[i % 4];
}

fn point_height(i: i32, count: i32, max_length: f32) -> f32 {
    return level_at(clamp(i, 0, count - 1)) * max_length;
}

fn effect(uv: vec2<f32>, size: vec2<f32>, opacity: f32, p0: vec4<f32>, p1: vec4<f32>, p2: vec4<f32>, p3: vec4<f32>) -> vec4<f32> {
    let color_a = p0.a;
    var color_rgb = vec3<f32>(0.0);
    if color_a > 0.0 {
        color_rgb = p0.rgb / color_a;
    }
    let n = p1.x;
    let vertical = p1.y;
    let alignment = p1.z;
    let max_length = p1.w;

    let p = uv * size;
    var x = p.x;
    var across = p.y;
    var along_size = size.x;
    var across_size = size.y;
    if vertical > 0.5 {
        x = p.y;
        across = p.x;
        along_size = size.y;
        across_size = size.x;
    }

    let count = i32(n);
    let last = count - 1;
    let step_x = along_size / (n - 1.0);

    let k = clamp(i32(floor(x / step_x + 0.5)), 0, last);
    let hk = point_height(k, count, max_length);
    var y: f32;
    var slope: f32;
    if k == 0 {
        let m0 = (hk + point_height(1, count, max_length)) * 0.5;
        slope = (m0 - hk) / (0.5 * step_x);
        y = hk + slope * x;
    } else if k == last {
        let m = (point_height(last - 1, count, max_length) + hk) * 0.5;
        let x0 = (f32(last) - 0.5) * step_x;
        slope = (hk - m) / (0.5 * step_x);
        y = m + slope * (x - x0);
    } else {
        let a = (point_height(k - 1, count, max_length) + hk) * 0.5;
        let b = (hk + point_height(k + 1, count, max_length)) * 0.5;
        let u = clamp((x - (f32(k) - 0.5) * step_x) / step_x, 0.0, 1.0);
        y = (1.0 - u) * (1.0 - u) * a + 2.0 * u * (1.0 - u) * hk + u * u * b;
        slope = (2.0 * (1.0 - u) * (hk - a) + 2.0 * u * (b - hk)) / step_x;
    }

    var t = across;
    if alignment > 1.5 {
        t = across_size - across;
    } else if alignment > 0.5 {
        t = abs(across - across_size * 0.5);
        y = y * 0.5;
        slope = slope * 0.5;
    }

    let d = (t - y) / sqrt(1.0 + slope * slope);
    let coverage = clamp(0.5 - d, 0.0, 1.0);
    if coverage <= 0.0 {
        return vec4<f32>(0.0);
    }
    return vec4<f32>(color_rgb, 1.0) * color_a * coverage * opacity;
}
