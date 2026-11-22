const MAX_LIGHT_COUNT: i32 = 128;
const POINT_LIGHT = 0u;
const SPOT_LIGHT = 1u;
const DIRECTIONAL_LIGHT = 2u;
const PI = 3.14159265359;
const SCATTERED = 0.5;

struct Billboard {
    right: vec4<f32>,
    up: vec4<f32>,
};

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
};

struct Light {
    position: vec3<f32>,
    intensity: f32,
    color: vec4<f32>,
    direction: vec3<f32>,
    light_type: u32,
    cos_cone_angle: f32,
    shadow_layer: i32,
    range: f32,
    shadow_far: f32,
    shadow_cascades: u32,
};

struct Lights {
    lights: array<Light, MAX_LIGHT_COUNT>,
    light_count: i32,
};

@group(0) @binding(0)
var<uniform> billboard: Billboard;

@group(1) @binding(0)
var<uniform> camera: CameraUniform;

@group(2) @binding(0)
var<uniform> lights: Lights;

struct VertexOutput {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) corner: vec2<f32>,
    @location(1) color: vec4<f32>,
    @location(2) world_position: vec3<f32>,
    @location(3) lit: f32,
};

@vertex
fn vs_main(
    @location(0) corner: vec2<f32>,
    @location(1) position_size: vec4<f32>,
    @location(2) color: vec4<f32>,
    @location(3) params: vec4<f32>,
) -> VertexOutput {
    let offset = (billboard.right.xyz * corner.x + billboard.up.xyz * corner.y)
        * position_size.w * 0.5;
    let world_position = position_size.xyz + offset;
    var out: VertexOutput;
    out.clip_position = camera.view_proj * vec4<f32>(world_position, 1.0);
    out.corner = corner;
    out.color = color;
    out.world_position = position_size.xyz;
    out.lit = params.x;
    return out;
}

fn ambient_light() -> vec3<f32> {
    if camera.sky_lit == 0u {
        return camera.ambient.rgb;
    }
    let overhead = camera.sky_irradiance[0].rgb * 0.2820948
        + camera.sky_irradiance[1].rgb * 0.4886025
        - camera.sky_irradiance[6].rgb * 0.3153916;
    return max(overhead, vec3<f32>(0.0)) / PI + camera.night_ambient.rgb;
}

fn direct_light(world_position: vec3<f32>) -> vec3<f32> {
    var total = vec3<f32>(0.0);
    for (var i: i32 = 0; i < min(lights.light_count, MAX_LIGHT_COUNT); i = i + 1) {
        let light = lights.lights[i];
        let light_delta = light.position - world_position;
        var attenuation = 1.0;
        if light.light_type != DIRECTIONAL_LIGHT {
            let distance_squared = dot(light_delta, light_delta);
            attenuation = 1.0 / max(distance_squared, 1e-4);
            if light.range > 0.0 {
                let ratio = distance_squared / (light.range * light.range);
                let window = clamp(1.0 - ratio * ratio, 0.0, 1.0);
                attenuation *= window * window;
            }
        }
        if light.light_type == SPOT_LIGHT {
            let along = dot(normalize(light_delta), -normalize(light.direction));
            let soft_edge = mix(light.cos_cone_angle, 1.0, 0.2);
            attenuation *= smoothstep(light.cos_cone_angle, soft_edge, along);
        }
        total += light.color.rgb * light.intensity * attenuation;
    }
    return total * SCATTERED / PI;
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    let from_center = dot(in.corner, in.corner);
    if from_center >= 1.0 {
        discard;
    }
    let soft = (1.0 - from_center) * (1.0 - from_center);
    var color = in.color.rgb;
    if in.lit > 0.5 {
        color *= ambient_light() + direct_light(in.world_position);
    }
    return vec4<f32>(color * soft, in.color.a * soft);
}
