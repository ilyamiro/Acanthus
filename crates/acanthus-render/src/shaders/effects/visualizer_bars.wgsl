fn level_at(i: i32) -> f32 {
    var v = data_at(u32(i / 4));
    return v[i % 4];
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
    let spacing = p1.w;
    let min_length = p2.x;
    let max_length = p2.y;
    let radius_ratio = p2.z;
    let opacity_base = p2.w;
    let opacity_range = p3.x;
    let edge_fade = p3.y;
    let ring_radius = p3.z;
    let ring_bar_width = p3.w;
    let tint_strength = ext0.x;

    let p = uv * size;
    var i: i32;
    var x: f32;
    var t: f32;
    var thickness: f32;
    var level: f32;
    var e = 1.0;
    var len: f32;
    var extent: f32;

    if ring_radius > 0.0 {
        let dir = p - size * 0.5;
        let step = 6.28318531 / n;
        var angle = atan2(dir.x, -dir.y);
        if angle < 0.0 {
            angle += 6.28318531;
        }
        i = i32(floor(angle / step + 0.5));
        if i >= i32(n) {
            i = 0;
        }
        let radial = vec2<f32>(sin(f32(i) * step), -cos(f32(i) * step));
        thickness = ring_bar_width;
        x = dot(dir, vec2<f32>(-radial.y, radial.x)) + thickness * 0.5;
        t = dot(dir, radial) - ring_radius;
        level = level_at(i);
        len = max(min_length, level * max_length);
        extent = len;
    } else {
        var along = p.x;
        var across = p.y;
        var along_size = size.x;
        var across_size = size.y;
        if vertical > 0.5 {
            along = p.y;
            across = p.x;
            along_size = size.y;
            across_size = size.x;
        }
        thickness = (along_size - (n - 1.0) * spacing) / n;
        let slot = thickness + spacing;
        i = i32(floor((along + spacing * 0.5) / slot));
        if i < 0 || i >= i32(n) {
            return vec4<f32>(0.0);
        }
        x = along - f32(i) * slot;
        level = level_at(i);
        if edge_fade > 0.5 {
            e = min(1.0, sin(f32(i) / max(1.0, n - 1.0) * 3.14159265) * 2.0);
            e = e * e * (3.0 - 2.0 * e);
            level = level * e;
        }
        len = max(min_length, level * max_length);
        t = across;
        extent = len;
        if alignment > 1.5 {
            t = across_size - across;
        } else if alignment > 0.5 {
            t = abs(across - across_size * 0.5);
            extent = len * 0.5;
        }
    }

    let r = min(thickness * radius_ratio, len * 0.5);
    var d: f32;
    if t > extent - r && (x < r || x > thickness - r) {
        let c = vec2<f32>(clamp(x, r, thickness - r), extent - r);
        d = length(vec2<f32>(x, t) - c) - r;
    } else {
        d = max(max(-x, x - thickness), t - extent);
    }
    if ring_radius > 0.0 {
        d = max(d, -t);
    }
    let coverage = clamp(0.5 - d, 0.0, 1.0);
    if coverage <= 0.0 {
        return vec4<f32>(0.0);
    }
    let rgb = mix(color_rgb, vec3<f32>(1.0), (f32(i) / n * 0.4 + level * 0.6) * tint_strength);
    return vec4<f32>(rgb, 1.0) * color_a * (opacity_base + level * opacity_range) * e * coverage * opacity;
}
