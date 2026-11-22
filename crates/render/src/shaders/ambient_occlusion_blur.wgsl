@group(0) @binding(0)
var t_occlusion: texture_2d<f32>;

@vertex
fn vs_main(@builtin(vertex_index) vertex_index: u32) -> @builtin(position) vec4<f32> {
    let uv = vec2<f32>(f32((vertex_index << 1u) & 2u), f32(vertex_index & 2u));
    return vec4<f32>(uv * 2.0 - 1.0, 0.0, 1.0);
}

@fragment
fn fs_main(@builtin(position) position: vec4<f32>) -> @location(0) vec4<f32> {
    let pixel = vec2<i32>(position.xy);
    let last = vec2<i32>(textureDimensions(t_occlusion)) - vec2<i32>(1);
    let center = textureLoad(t_occlusion, pixel, 0).rg;
    let tolerance = 0.02 + center.g * 0.03;

    var total = 0.0;
    var weight = 0.0;
    for (var y = -2; y < 2; y = y + 1) {
        for (var x = -2; x < 2; x = x + 1) {
            let texel = textureLoad(t_occlusion, clamp(pixel + vec2<i32>(x, y), vec2<i32>(0), last), 0).rg;
            let near = max(1.0 - abs(texel.g - center.g) / tolerance, 0.0);
            total += texel.r * near;
            weight += near;
        }
    }
    return vec4<f32>(total / max(weight, 1e-4), 0.0, 0.0, 1.0);
}
