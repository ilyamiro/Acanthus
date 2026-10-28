struct Adjust {
    color: vec4<f32>,
    vals: vec4<f32>,
    mask: vec4<f32>,
};

@group(0) @binding(0) var<uniform> adjust: Adjust;
@group(1) @binding(0) var source: texture_2d<f32>;
@group(2) @binding(0) var mask_source: texture_2d<f32>;

@vertex
fn vs_main(@builtin(vertex_index) vi: u32) -> @builtin(position) vec4<f32> {
    let corner = vec2<f32>(f32(vi & 1u), f32(vi >> 1u));
    return vec4<f32>(corner.x * 2.0 - 1.0, 1.0 - corner.y * 2.0, 0.0, 1.0);
}

@fragment
fn fs_main(@builtin(position) pos: vec4<f32>) -> @location(0) vec4<f32> {
    let p = vec2<i32>(pos.xy);
    let texel = textureLoad(source, p, 0);
    var a = texel.a;
    if a <= 0.0 {
        return vec4<f32>(0.0);
    }
    var rgb = texel.rgb / a;
    rgb = rgb + vec3<f32>(adjust.vals.x);
    rgb = (rgb - vec3<f32>(0.5)) * (1.0 + adjust.vals.y) + vec3<f32>(0.5);
    let luma = dot(rgb, vec3<f32>(0.299, 0.587, 0.114));
    rgb = mix(vec3<f32>(luma), rgb, 1.0 + adjust.vals.z);
    rgb = mix(rgb, adjust.color.rgb * luma, adjust.vals.w);
    rgb = clamp(rgb, vec3<f32>(0.0), vec3<f32>(1.0));
    if adjust.mask.x > 0.5 {
        let m = textureLoad(mask_source, p, 0).a;
        var cut = m;
        if adjust.mask.y > 0.0 || adjust.mask.z > 0.0 {
            cut = smoothstep(adjust.mask.y, adjust.mask.y + max(adjust.mask.z, 0.0001), m);
        }
        a = a * cut;
    }
    return vec4<f32>(rgb * a, a);
}
