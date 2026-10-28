const HALF_PI: f32 = 1.5707963;

fn sign_or_one(v: vec2<f32>) -> vec2<f32> {
    return select(vec2<f32>(-1.0), vec2<f32>(1.0), v >= vec2<f32>(0.0));
}

fn effect(uv: vec2<f32>, size: vec2<f32>, opacity: f32, p0: vec4<f32>, p1: vec4<f32>, p2: vec4<f32>, p3: vec4<f32>) -> vec4<f32> {
    let radius = p1.x;
    let inset = p1.y;
    let line_width = p1.z;
    let progress = p1.w;
    let p = uv * size;
    let half_size = size * 0.5 - vec2<f32>(inset);
    let q = p - size * 0.5;
    let b = max(half_size - vec2<f32>(radius), vec2<f32>(0.0));
    let a = abs(q);
    let e = a - b;
    let dist = length(max(e, vec2<f32>(0.0))) + min(max(e.x, e.y), 0.0) - radius;
    let sx = 2.0 * b.x;
    let sy = 2.0 * b.y;
    let arc = HALF_PI * radius;
    let perimeter = 2.0 * sx + 2.0 * sy + 4.0 * arc;
    var s: f32;
    if e.x > 0.0 && e.y > 0.0 {
        let c = q - sign_or_one(q) * b;
        if q.x > 0.0 && q.y < 0.0 {
            s = sx + atan2(c.x, -c.y + 1e-6) * radius;
        } else if q.x > 0.0 {
            s = sx + sy + arc + atan2(c.y, c.x + 1e-6) * radius;
        } else if q.y > 0.0 {
            s = 2.0 * sx + sy + 2.0 * arc + atan2(-c.x, c.y + 1e-6) * radius;
        } else {
            s = 2.0 * sx + 2.0 * sy + 3.0 * arc + atan2(-c.y, -c.x + 1e-6) * radius;
        }
    } else if e.x > e.y {
        if q.x > 0.0 {
            s = sx + arc + (q.y + b.y);
        } else {
            s = 2.0 * sx + sy + 3.0 * arc + (b.y - q.y);
        }
    } else {
        if q.y < 0.0 {
            s = q.x + b.x;
        } else {
            s = sx + sy + 2.0 * arc + (b.x - q.x);
        }
    }
    let from_start = s - (2.0 * sx + sy + 3.0 * arc);
    let along = from_start + select(0.0, perimeter, from_start < 0.0);
    let drawn = clamp(progress - along + 0.5, 0.0, 1.0);
    let coverage = clamp(line_width * 0.5 + 0.5 - abs(dist), 0.0, 1.0);
    return vec4<f32>(1.0) * coverage * drawn * opacity;
}
