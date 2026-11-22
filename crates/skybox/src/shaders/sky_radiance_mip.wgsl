@group(0) @binding(0)
var t_source: texture_2d<f32>;

@vertex
fn vs_main(@builtin(vertex_index) vertex_index: u32) -> @builtin(position) vec4<f32> {
    let uv = vec2<f32>(f32((vertex_index << 1u) & 2u), f32(vertex_index & 2u));
    return vec4<f32>(uv * 2.0 - 1.0, 0.0, 1.0);
}

fn radiance(pixel: vec2<i32>) -> vec3<f32> {
    let last = vec2<i32>(textureDimensions(t_source)) - vec2<i32>(1);
    let texel = textureLoad(t_source, clamp(pixel, vec2<i32>(0), last), 0).rgb;
#ifdef SKY_RADIANCE_SQRT
    return texel * texel;
#else
    return texel;
#endif
}

@fragment
fn fs_main(@builtin(position) position: vec4<f32>) -> @location(0) vec4<f32> {
    let pixel = vec2<i32>(position.xy) * 2;
    let average = (radiance(pixel)
        + radiance(pixel + vec2<i32>(1, 0))
        + radiance(pixel + vec2<i32>(0, 1))
        + radiance(pixel + vec2<i32>(1, 1))) * 0.25;
#ifdef SKY_RADIANCE_SQRT
    return vec4<f32>(sqrt(average), 1.0);
#else
    return vec4<f32>(average, 1.0);
#endif
}
