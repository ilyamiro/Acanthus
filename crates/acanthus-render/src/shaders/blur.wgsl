struct Blur {
    dir: vec2<f32>,
    radius: f32,
    sigma: f32,
    tint: vec4<f32>,
};

@group(0) @binding(0) var<uniform> blur: Blur;
@group(1) @binding(0) var source: texture_2d<f32>;

@vertex
fn vs_main(@builtin(vertex_index) vi: u32) -> @builtin(position) vec4<f32> {
    let corner = vec2<f32>(f32(vi & 1u), f32(vi >> 1u));
    return vec4<f32>(corner.x * 2.0 - 1.0, 1.0 - corner.y * 2.0, 0.0, 1.0);
}

@fragment
fn fs_main(@builtin(position) pos: vec4<f32>) -> @location(0) vec4<f32> {
    let origin = vec2<i32>(pos.xy);
    let size = vec2<i32>(textureDimensions(source));
    let taps = i32(ceil(blur.radius));
    let stride = vec2<i32>(blur.dir);
    let inv = 1.0 / (2.0 * blur.sigma * blur.sigma);
    var sum = vec4<f32>(0.0);
    var total = 0.0;
    for (var i = -taps; i <= taps; i++) {
        let w = exp(-f32(i * i) * inv);
        total += w;
        let q = origin + stride * i;
        if q.x >= 0 && q.y >= 0 && q.x < size.x && q.y < size.y {
            sum += textureLoad(source, q, 0) * w;
        }
    }
    let v = sum / total;
    if blur.tint.w >= 0.0 {
        return blur.tint * v.a;
    }
    return v;
}
