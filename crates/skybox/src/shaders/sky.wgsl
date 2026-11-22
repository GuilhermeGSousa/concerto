const PI: f32 = 3.14159265358979;
const LIMB_DARKENING: f32 = 0.6;

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

struct Sun {
    toward_sun: vec4<f32>,
    disk_radiance: vec4<f32>,
};

@group(0) @binding(0)
var<uniform> camera: CameraUniform;

@group(0) @binding(1)
var t_sky_radiance: texture_2d<f32>;

@group(0) @binding(2)
var s_sky_radiance: sampler;

@group(1) @binding(0)
var<uniform> sun: Sun;

struct VertexOutput {
    @builtin(position) clip: vec4<f32>,
    @location(0) ndc: vec2<f32>,
};

@vertex
fn vs_main(@builtin(vertex_index) vertex_index: u32) -> VertexOutput {
    let corner = vec2<f32>(f32((vertex_index << 1u) & 2u), f32(vertex_index & 2u));
    var out: VertexOutput;
    out.ndc = corner * 2.0 - 1.0;
    out.clip = vec4<f32>(out.ndc, 0.0, 1.0);
    return out;
}

#ifdef SKY_RADIANCE_SQRT
const SKY_RADIANCE_SCALE: f32 = f32(#{SKY_RADIANCE_SQRT});
#endif

fn sky_radiance_uv(direction: vec3<f32>) -> vec2<f32> {
    return vec2<f32>(
        atan2(direction.x, -direction.z) / (2.0 * PI) + 0.5,
        0.5 - asin(clamp(direction.y, -1.0, 1.0)) / PI,
    );
}

fn sky_radiance(direction: vec3<f32>) -> vec3<f32> {
    let texel = textureSampleLevel(t_sky_radiance, s_sky_radiance, sky_radiance_uv(direction), 0.0).rgb;
#ifdef SKY_RADIANCE_SQRT
    return texel * texel * SKY_RADIANCE_SCALE;
#else
    return texel;
#endif
}

fn sun_disk(direction: vec3<f32>) -> vec3<f32> {
    let cos_disk = sun.toward_sun.w;
    let cos_angle = dot(direction, sun.toward_sun.xyz);
    if cos_angle < cos_disk || direction.y < sun.disk_radiance.w {
        return vec3<f32>(0.0);
    }
    let edge = acos(clamp(cos_disk, -1.0, 1.0));
    let from_center = acos(clamp(cos_angle, -1.0, 1.0)) / max(edge, 1.0e-6);
    let mu = sqrt(max(1.0 - from_center * from_center, 0.0));
    let limb = (1.0 - LIMB_DARKENING * (1.0 - mu)) / (1.0 - LIMB_DARKENING / 3.0);
    return sun.disk_radiance.rgb * limb;
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    let far = camera.inverse_view_proj * vec4<f32>(in.ndc, 0.5, 1.0);
    let direction = normalize(far.xyz / far.w - camera.view_pos);
    let radiance = sky_radiance(direction) + sun_disk(direction) + camera.night_color.rgb;
    return vec4<f32>(radiance, 1.0);
}
