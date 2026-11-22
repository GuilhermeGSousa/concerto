const KEY_LOG2 = -2.4739;
const DARKEST_LOG2 = -13.0;

struct ExposureUniform {
    rect: vec4<f32>,
    min_stops: f32,
    max_stops: f32,
    _padding: vec2<f32>,
};

@group(0) @binding(0)
var t_source: texture_2d<f32>;

@group(0) @binding(1)
var s_source: sampler;

@group(0) @binding(2)
var<uniform> exposure: ExposureUniform;

@vertex
fn vs_main(@builtin(vertex_index) vertex_index: u32) -> @builtin(position) vec4<f32> {
    let uv = vec2<f32>(f32((vertex_index << 1u) & 2u), f32(vertex_index & 2u));
    return vec4<f32>(uv * 2.0 - 1.0, 0.0, 1.0);
}

@fragment
fn fs_measure(@builtin(position) position: vec4<f32>) -> @location(0) vec4<f32> {
    let cell = floor(position.xy);
    var total = 0.0;
    for (var y = 0; y < 4; y = y + 1) {
        for (var x = 0; x < 4; x = x + 1) {
            let within = (cell + (vec2<f32>(f32(x), f32(y)) + vec2<f32>(0.5)) / 4.0) / 256.0;
            let uv = exposure.rect.xy + within * exposure.rect.zw;
            let light = max(textureSampleLevel(t_source, s_source, uv, 0.0).rgb, vec3<f32>(0.0));
            let luminance = dot(light, vec3<f32>(0.2126, 0.7152, 0.0722));
            total += max(log2(luminance), DARKEST_LOG2);
        }
    }
    return vec4<f32>(total / 16.0, 0.0, 0.0, 1.0);
}

@fragment
fn fs_reduce(@builtin(position) position: vec4<f32>) -> @location(0) vec4<f32> {
    let cell = vec2<i32>(position.xy) * 4;
    var total = 0.0;
    for (var y = 0; y < 4; y = y + 1) {
        for (var x = 0; x < 4; x = x + 1) {
            total += textureLoad(t_source, cell + vec2<i32>(x, y), 0).r;
        }
    }
    return vec4<f32>(total / 16.0, 0.0, 0.0, 1.0);
}

@fragment
fn fs_adapt(@builtin(position) position: vec4<f32>) -> @location(0) vec4<f32> {
    let measured = textureLoad(t_source, vec2<i32>(0), 0).r;
    let stops = clamp(KEY_LOG2 - measured, exposure.min_stops, exposure.max_stops);
    return vec4<f32>(stops, 0.0, 0.0, 1.0);
}
