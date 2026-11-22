const HAS_BASE_COLOR_TEXTURE = 1u << 0u;

struct MaterialUniform {
    base_color_factor: vec4<f32>,
    emissive_factor: vec3<f32>,
    metallic_factor: f32,
    roughness_factor: f32,
    occlusion_strength: f32,
    flags: u32,
    alpha_cutoff: f32,
    uv_scale: vec2<f32>,
    _padding: vec2<u32>,
}

@group(2) @binding(0)
var t_base_color: texture_2d<f32>;
@group(2) @binding(1)
var s_base_color: sampler;
@group(2) @binding(10)
var<uniform> material: MaterialUniform;

@fragment
fn fs_cutout(@location(0) tex_coords: vec2<f32>) {
    var alpha = material.base_color_factor.a;
    if (material.flags & HAS_BASE_COLOR_TEXTURE) != 0u {
        alpha *= textureSample(t_base_color, s_base_color, tex_coords * material.uv_scale).a;
    }
    if alpha < material.alpha_cutoff {
        discard;
    }
}
