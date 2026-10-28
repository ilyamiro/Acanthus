fn reveal_at(p: vec2<f32>, size: vec2<f32>, progress: f32, kind: f32, origin: vec2<f32>, params: vec4<f32>) -> f32 {
    if progress < 0.0 {
        return 0.0;
    }
    let clamped = clamp(progress, 0.0, 1.0);
    if clamped >= 1.0 {
        return 1.0;
    }
    if kind < 0.5 {
        return clamped;
    }
    if kind < 1.5 {
        var d = abs(p.x - size.x * 0.5);
        var half_span = clamped * size.x * 0.5;
        if params.x > 0.5 {
            d = abs(p.y - size.y * 0.5);
            half_span = clamped * size.y * 0.5;
        }
        return clamp(half_span - d + 0.5, 0.0, 1.0);
    }
    if kind < 2.5 {
        let center = origin * size;
        let d1 = length(center);
        let d2 = length(vec2<f32>(size.x - center.x, center.y));
        let d3 = length(vec2<f32>(center.x, size.y - center.y));
        let d4 = length(size - center);
        let max_r = max(max(d1, d2), max(d3, d4));
        return clamp(clamped * max_r - length(p - center) + 0.5, 0.0, 1.0);
    }
    let dir = i32(floor(params.y + 0.5));
    var dist = 0.0;
    var total = 1.0;
    if dir == 0 {
        dist = p.x;
        total = size.x;
    } else if dir == 1 {
        dist = size.x - p.x;
        total = size.x;
    } else if dir == 2 {
        dist = p.y;
        total = size.y;
    } else if dir == 3 {
        dist = size.y - p.y;
        total = size.y;
    } else {
        var dir_vec = normalize(-size);
        var origin_pt = size;
        if dir == 4 {
            origin_pt = vec2<f32>(0.0, 0.0);
            dir_vec = normalize(size);
        } else if dir == 5 {
            origin_pt = vec2<f32>(size.x, 0.0);
            dir_vec = normalize(vec2<f32>(-size.x, size.y));
        } else if dir == 6 {
            origin_pt = vec2<f32>(0.0, size.y);
            dir_vec = normalize(vec2<f32>(size.x, -size.y));
        }
        dist = dot(p - origin_pt, dir_vec);
        total = length(size);
    }
    return clamp(clamped * total - dist + 0.5, 0.0, 1.0);
}

fn effect(uv: vec2<f32>, size: vec2<f32>, opacity: f32, p0: vec4<f32>, p1: vec4<f32>, p2: vec4<f32>, p3: vec4<f32>) -> vec4<f32> {
    let m = reveal_at(uv * size, size, p0.x, p0.y, p0.zw, p1);
    return mix(sample0(uv), sample1(uv), m) * opacity;
}
