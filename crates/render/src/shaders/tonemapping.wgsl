const TONEMAPPING_NONE = 0u;
const TONEMAPPING_ACES = 1u;

struct CameraUniform {
    view_pos: vec3<f32>,
    view_proj: mat4x4<f32>,
    fog_color: vec4<f32>,
    fog_params: vec4<f32>,
    ambient: vec4<f32>,
    inverse_view_proj: mat4x4<f32>,
    sky_irradiance: array<vec4<f32>, 9>,
    night_color: vec4<f32>,
    night_ambient: vec4<f32>,
    sky_lit: u32,
    exposure: f32,
    tonemapping: u32,
};

@group(0) @binding(0)
var<uniform> camera: CameraUniform;

@group(1) @binding(0)
var t_scene: texture_2d<f32>;

@vertex
fn vs_main(@builtin(vertex_index) vertex_index: u32) -> @builtin(position) vec4<f32> {
    let uv = vec2<f32>(f32((vertex_index << 1u) & 2u), f32(vertex_index & 2u));
    return vec4<f32>(uv * 2.0 - 1.0, 0.0, 1.0);
}

fn aces_tonemap(color: vec3<f32>) -> vec3<f32> {
    let mapped = (color * (2.51 * color + 0.03)) / (color * (2.43 * color + 0.59) + 0.14);
    return clamp(mapped, vec3<f32>(0.0), vec3<f32>(1.0));
}

@fragment
fn fs_main(@builtin(position) position: vec4<f32>) -> @location(0) vec4<f32> {
    let scene = textureLoad(t_scene, vec2<i32>(position.xy), 0);
    let exposed = max(scene.rgb, vec3<f32>(0.0)) * camera.exposure;
    var color = clamp(exposed, vec3<f32>(0.0), vec3<f32>(1.0));
    if camera.tonemapping == TONEMAPPING_ACES {
        color = aces_tonemap(exposed);
    }
    return vec4<f32>(color, scene.a);
}
