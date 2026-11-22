const PI = 3.14159265359;
const HALF_PI = 1.57079632679;
const SLICES = 3;
const STEPS = 4;
const MAX_RADIUS_PIXELS = 96.0;
const FALLOFF = 0.4;

struct CameraUniform {
    view_pos: vec3<f32>,
    view_proj: mat4x4<f32>,
    fog_color: vec4<f32>,
    fog_params: vec4<f32>,
    ambient: vec4<f32>,
    inverse_view_proj: mat4x4<f32>,
};

struct OcclusionUniform {
    rect: vec4<f32>,
    right: vec3<f32>,
    pixels_per_unit: f32,
    up: vec3<f32>,
    radius: f32,
    forward: vec3<f32>,
    intensity: f32,
};

struct Occlusion {
    @location(0) value: vec4<f32>,
};

@group(0) @binding(0)
var<uniform> camera: CameraUniform;

@group(1) @binding(0)
var t_depth: texture_2d<f32>;

@group(1) @binding(1)
var<uniform> occlusion: OcclusionUniform;

@vertex
fn vs_main(@builtin(vertex_index) vertex_index: u32) -> @builtin(position) vec4<f32> {
    let uv = vec2<f32>(f32((vertex_index << 1u) & 2u), f32(vertex_index & 2u));
    return vec4<f32>(uv * 2.0 - 1.0, 0.0, 1.0);
}

fn depth_at(pixel: vec2<i32>) -> f32 {
    let low = vec2<i32>(occlusion.rect.xy);
    let high = low + vec2<i32>(occlusion.rect.zw) - vec2<i32>(1);
    return textureLoad(t_depth, clamp(pixel, low, high), 0).r;
}

fn world_at(pixel: vec2<f32>, depth: f32) -> vec3<f32> {
    let uv = (pixel - occlusion.rect.xy) / occlusion.rect.zw;
    let ndc = vec2<f32>(uv.x * 2.0 - 1.0, 1.0 - uv.y * 2.0);
    let world = camera.inverse_view_proj * vec4<f32>(ndc, depth, 1.0);
    return world.xyz / world.w;
}

fn position_at(pixel: vec2<i32>) -> vec3<f32> {
    return world_at(vec2<f32>(pixel) + vec2<f32>(0.5), depth_at(pixel));
}

fn view_depth(position: vec3<f32>) -> f32 {
    return dot(position - camera.view_pos, occlusion.forward);
}

fn surface_normal(pixel: vec2<i32>, position: vec3<f32>, toward_eye: vec3<f32>) -> vec3<f32> {
    let depth = view_depth(position);
    let left = position_at(pixel - vec2<i32>(1, 0));
    let right = position_at(pixel + vec2<i32>(1, 0));
    let above = position_at(pixel - vec2<i32>(0, 1));
    let below = position_at(pixel + vec2<i32>(0, 1));

    var across = right - position;
    if abs(view_depth(left) - depth) < abs(view_depth(right) - depth) {
        across = position - left;
    }
    var down = below - position;
    if abs(view_depth(above) - depth) < abs(view_depth(below) - depth) {
        down = position - above;
    }
    let normal = normalize(cross(down, across));
    return select(normal, -normal, dot(normal, toward_eye) < 0.0);
}

fn interleaved_noise(pixel: vec2<f32>) -> f32 {
    return fract(52.9829189 * fract(dot(pixel, vec2<f32>(0.06711056, 0.00583715))));
}

fn horizon(
    pixel: vec2<f32>,
    reach: vec2<f32>,
    jitter: f32,
    position: vec3<f32>,
    toward_eye: vec3<f32>,
    lowest: f32,
) -> f32 {
    var highest = lowest;
    for (var index = 0; index < STEPS; index = index + 1) {
        let along = (f32(index) + jitter) / f32(STEPS);
        let offset = reach * along * along;
        let sample_pixel = vec2<i32>(floor(pixel + offset));
        if all(sample_pixel == vec2<i32>(floor(pixel))) {
            continue;
        }
        let depth = depth_at(sample_pixel);
        if depth >= 1.0 {
            continue;
        }
        let to_sample = world_at(vec2<f32>(sample_pixel) + vec2<f32>(0.5), depth) - position;
        let distance = length(to_sample);
        if distance < 1e-5 {
            continue;
        }
        let weight = clamp((1.0 - distance / occlusion.radius) / FALLOFF, 0.0, 1.0);
        let cosine = dot(to_sample / distance, toward_eye);
        highest = max(highest, mix(lowest, cosine, weight));
    }
    return highest;
}

fn slice_visibility(normal_angle: f32, horizon_angle: f32) -> f32 {
    return (cos(normal_angle) + 2.0 * horizon_angle * sin(normal_angle)
        - cos(2.0 * horizon_angle - normal_angle)) * 0.25;
}

@fragment
fn fs_main(@builtin(position) frag_position: vec4<f32>) -> Occlusion {
    var out: Occlusion;
    let pixel = vec2<i32>(frag_position.xy);
    let depth = depth_at(pixel);
    if depth >= 1.0 {
        out.value = vec4<f32>(1.0, 60000.0, 0.0, 1.0);
        return out;
    }

    let position = world_at(frag_position.xy, depth);
    let eye_depth = view_depth(position);
    let toward_eye = normalize(camera.view_pos - position);
    let normal = surface_normal(pixel, position, toward_eye);

    let radius_pixels = min(
        occlusion.radius * occlusion.pixels_per_unit / max(eye_depth, 1e-3),
        MAX_RADIUS_PIXELS,
    );
    if radius_pixels < 1.0 {
        out.value = vec4<f32>(1.0, eye_depth, 0.0, 1.0);
        return out;
    }

    let noise = interleaved_noise(frag_position.xy);
    let jitter = interleaved_noise(frag_position.xy + vec2<f32>(47.0, 17.0));
    var visibility = 0.0;
    for (var slice = 0; slice < SLICES; slice = slice + 1) {
        let angle = (f32(slice) + noise) / f32(SLICES) * PI;
        let screen = vec2<f32>(cos(angle), sin(angle));
        let direction = occlusion.right * screen.x - occlusion.up * screen.y;
        let flat_direction = direction - toward_eye * dot(direction, toward_eye);
        let axis = normalize(cross(flat_direction, toward_eye));
        let in_slice = normal - axis * dot(normal, axis);
        let in_slice_length = length(in_slice);
        if in_slice_length < 1e-4 {
            visibility += 1.0;
            continue;
        }
        let cos_normal = clamp(dot(in_slice, toward_eye) / in_slice_length, -1.0, 1.0);
        let normal_angle = sign(dot(flat_direction, in_slice)) * acos(cos_normal);

        let reach = screen * radius_pixels;
        let ahead = horizon(frag_position.xy, reach, jitter, position, toward_eye, cos(normal_angle + HALF_PI));
        let behind = horizon(frag_position.xy, -reach, jitter, position, toward_eye, cos(normal_angle - HALF_PI));

        let ahead_angle = normal_angle + min(acos(clamp(ahead, -1.0, 1.0)) - normal_angle, HALF_PI);
        let behind_angle = normal_angle + max(-acos(clamp(behind, -1.0, 1.0)) - normal_angle, -HALF_PI);
        visibility += in_slice_length
            * (slice_visibility(normal_angle, ahead_angle) + slice_visibility(normal_angle, behind_angle));
    }
    visibility = clamp(visibility / f32(SLICES), 0.0, 1.0);

    out.value = vec4<f32>(mix(1.0, visibility, occlusion.intensity), eye_depth, 0.0, 1.0);
    return out;
}
