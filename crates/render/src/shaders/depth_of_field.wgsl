const TAPS = 32;
const GOLDEN_ANGLE = 2.39996323;

struct CameraUniform {
    view_pos: vec3<f32>,
    view_proj: mat4x4<f32>,
    fog_color: vec4<f32>,
    fog_params: vec4<f32>,
    ambient: vec4<f32>,
    inverse_view_proj: mat4x4<f32>,
};

struct FocusUniform {
    rect: vec4<f32>,
    forward: vec3<f32>,
    focus_distance: f32,
    blur_scale: f32,
    max_blur: f32,
    _padding: vec2<f32>,
};

@group(0) @binding(0)
var<uniform> camera: CameraUniform;

@group(1) @binding(0)
var<uniform> focus: FocusUniform;

#ifdef MULTISAMPLED_DEPTH
@group(1) @binding(1)
var t_depth: texture_depth_multisampled_2d;
#else
@group(1) @binding(1)
var t_depth: texture_2d<f32>;
#endif

@group(1) @binding(2)
var t_scene: texture_2d<f32>;

@group(1) @binding(3)
var s_scene: sampler;

@group(1) @binding(4)
var t_blur: texture_2d<f32>;

@vertex
fn vs_main(@builtin(vertex_index) vertex_index: u32) -> @builtin(position) vec4<f32> {
    let uv = vec2<f32>(f32((vertex_index << 1u) & 2u), f32(vertex_index & 2u));
    return vec4<f32>(uv * 2.0 - 1.0, 0.0, 1.0);
}

fn depth_at(pixel: vec2<i32>) -> f32 {
#ifdef MULTISAMPLED_DEPTH
    return textureLoad(t_depth, pixel, 0);
#else
    return textureLoad(t_depth, pixel, 0).r;
#endif
}

@fragment
fn fs_blur_radius(@builtin(position) position: vec4<f32>) -> @location(0) vec4<f32> {
    let depth = depth_at(vec2<i32>(position.xy));
    let uv = (position.xy - focus.rect.xy) / focus.rect.zw;
    let ndc = vec2<f32>(uv.x * 2.0 - 1.0, 1.0 - uv.y * 2.0);
    let world = camera.inverse_view_proj * vec4<f32>(ndc, depth, 1.0);
    let distance = max(dot(world.xyz / world.w - camera.view_pos, focus.forward), 1e-3);
    let radius = focus.blur_scale * (distance - focus.focus_distance) / distance;
    return vec4<f32>(clamp(radius, -focus.max_blur, focus.max_blur), 0.0, 0.0, 1.0);
}

@fragment
fn fs_blur(@builtin(position) position: vec4<f32>) -> @location(0) vec4<f32> {
    let pixel = vec2<i32>(position.xy);
    let size = vec2<f32>(textureDimensions(t_scene));
    let last = vec2<i32>(size) - vec2<i32>(1);
    let center = textureLoad(t_scene, pixel, 0);
    let own = textureLoad(t_blur, pixel, 0).r;
    let reach = abs(own);
    if reach < 0.5 {
        return center;
    }

    var total = center.rgb;
    var weight = 1.0;
    for (var index = 0; index < TAPS; index = index + 1) {
        let turn = f32(index) * GOLDEN_ANGLE;
        let away = sqrt((f32(index) + 0.5) / f32(TAPS)) * reach;
        let offset = vec2<f32>(cos(turn), sin(turn)) * away;
        let tap = clamp(vec2<i32>(position.xy + offset), vec2<i32>(0), last);
        let theirs = textureLoad(t_blur, tap, 0).r;
        var spread = 1.0;
        if theirs < own {
            spread = clamp(abs(theirs) - away + 1.0, 0.0, 1.0);
        }
        total += textureSampleLevel(t_scene, s_scene, (position.xy + offset) / size, 0.0).rgb * spread;
        weight += spread;
    }
    return vec4<f32>(total / weight, center.a);
}
