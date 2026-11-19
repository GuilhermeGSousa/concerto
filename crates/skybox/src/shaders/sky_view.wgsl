const PI: f32 = 3.14159265358979;
const SKY_STEPS: u32 = 32u;

struct SkyView {
    rayleigh_scattering: vec4<f32>,
    ozone_absorption: vec4<f32>,
    mie: vec4<f32>,
    planet: vec4<f32>,
    toward_sun: vec4<f32>,
    illuminance: vec4<f32>,
    lut_sizes: vec4<f32>,
};

@group(0) @binding(0)
var<uniform> sky: SkyView;

@group(0) @binding(1)
var t_transmittance: texture_2d<f32>;

@group(0) @binding(2)
var t_multiple_scattering: texture_2d<f32>;

@group(0) @binding(3)
var s_lut: sampler;

struct VertexOutput {
    @builtin(position) clip: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

@vertex
fn vs_main(@builtin(vertex_index) vertex_index: u32) -> VertexOutput {
    let corner = vec2<f32>(f32((vertex_index << 1u) & 2u), f32(vertex_index & 2u));
    var out: VertexOutput;
    out.clip = vec4<f32>(corner * 2.0 - 1.0, 0.0, 1.0);
    out.uv = vec2<f32>(corner.x, 1.0 - corner.y);
    return out;
}

struct Medium {
    rayleigh_scattering: vec3<f32>,
    mie_scattering: f32,
    extinction: vec3<f32>,
};

fn planet_radius() -> f32 {
    return sky.planet.x;
}

fn top_radius() -> f32 {
    return sky.planet.y;
}

fn medium(height_above_ground: f32) -> Medium {
    let height = max(height_above_ground, 0.0);
    let rayleigh = exp(-height / sky.rayleigh_scattering.w);
    let mie = exp(-height / sky.mie.z);
    let ozone = max(
        1.0 - abs(height - sky.ozone_absorption.w) / max(0.5 * sky.planet.w, 1.0e-7),
        0.0,
    );
    var air: Medium;
    air.rayleigh_scattering = sky.rayleigh_scattering.xyz * rayleigh;
    air.mie_scattering = sky.mie.x * mie;
    air.extinction = air.rayleigh_scattering
        + vec3<f32>((sky.mie.x + sky.mie.y) * mie)
        + sky.ozone_absorption.xyz * ozone;
    return air;
}

fn rayleigh_phase(cos_theta: f32) -> f32 {
    return 3.0 / (16.0 * PI) * (1.0 + cos_theta * cos_theta);
}

fn mie_phase(g: f32, cos_theta: f32) -> f32 {
    let g2 = g * g;
    let k = 3.0 / (8.0 * PI) * (1.0 - g2) / (2.0 + g2);
    let denominator = max(1.0 + g2 - 2.0 * g * cos_theta, 1.0e-6);
    return k * (1.0 + cos_theta * cos_theta) / (denominator * sqrt(denominator));
}

fn distance_to_top(radius: f32, mu: f32) -> f32 {
    let top = top_radius();
    let discriminant = radius * radius * (mu * mu - 1.0) + top * top;
    return max(-radius * mu + sqrt(max(discriminant, 0.0)), 0.0);
}

fn ground_discriminant(radius: f32, mu: f32) -> f32 {
    let bottom = planet_radius();
    return radius * radius * (mu * mu - 1.0) + bottom * bottom;
}

fn hits_ground(radius: f32, mu: f32) -> bool {
    return mu < 0.0 && ground_discriminant(radius, mu) >= 0.0;
}

fn distance_to_ground(radius: f32, mu: f32) -> f32 {
    return max(-radius * mu - sqrt(max(ground_discriminant(radius, mu), 0.0)), 0.0);
}

fn lut_uv(unit: vec2<f32>, size: vec2<f32>) -> vec2<f32> {
    return (unit * (size - 1.0) + 0.5) / size;
}

fn transmittance_lut_unit(radius: f32, mu: f32) -> vec2<f32> {
    let bottom = planet_radius();
    let top = top_radius();
    let horizon = sqrt(max(top * top - bottom * bottom, 0.0));
    let rho = sqrt(max(radius * radius - bottom * bottom, 0.0));
    let distance = distance_to_top(radius, mu);
    let shortest = top - radius;
    let longest = rho + horizon;
    let u = (distance - shortest) / max(longest - shortest, 1.0e-7);
    let v = rho / max(horizon, 1.0e-7);
    return clamp(vec2<f32>(u, v), vec2<f32>(0.0), vec2<f32>(1.0));
}

fn sun_transmittance(radius: f32, mu: f32) -> vec3<f32> {
    if hits_ground(radius, mu) {
        return vec3<f32>(0.0);
    }
    let uv = lut_uv(transmittance_lut_unit(radius, mu), sky.lut_sizes.xy);
    return textureSampleLevel(t_transmittance, s_lut, uv, 0.0).rgb;
}

fn multiple_scattering(radius: f32, sun_mu: f32) -> vec3<f32> {
    let unit = clamp(
        vec2<f32>(
            sun_mu * 0.5 + 0.5,
            (radius - planet_radius()) / max(top_radius() - planet_radius(), 1.0e-7),
        ),
        vec2<f32>(0.0),
        vec2<f32>(1.0),
    );
    let uv = lut_uv(unit, sky.lut_sizes.zw);
    return textureSampleLevel(t_multiple_scattering, s_lut, uv, 0.0).rgb;
}

fn integrate_step(source: vec3<f32>, extinction: vec3<f32>, step_transmittance: vec3<f32>) -> vec3<f32> {
    return (source - source * step_transmittance) / max(extinction, vec3<f32>(1.0e-9));
}

fn sky_radiance(view: vec3<f32>, sun: vec3<f32>) -> vec3<f32> {
    let radius = sky.planet.z;
    let origin = vec3<f32>(0.0, radius, 0.0);
    var ray_length = distance_to_top(radius, view.y);
    if hits_ground(radius, view.y) {
        ray_length = distance_to_ground(radius, view.y);
    }
    let cos_theta = dot(view, sun);
    let rayleigh = rayleigh_phase(cos_theta);
    let mie = mie_phase(sky.mie.w, cos_theta);
    var throughput = vec3<f32>(1.0);
    var radiance = vec3<f32>(0.0);
    for (var index = 0u; index < SKY_STEPS; index++) {
        let start_fraction = f32(index) / f32(SKY_STEPS);
        let end_fraction = f32(index + 1u) / f32(SKY_STEPS);
        let start = start_fraction * start_fraction * ray_length;
        let end = end_fraction * end_fraction * ray_length;
        let step = end - start;
        let position = origin + view * (start + 0.3 * step);
        let sample_radius = length(position);
        let sun_mu = dot(position, sun) / sample_radius;
        let air = medium(sample_radius - planet_radius());
        let lit = sun_transmittance(sample_radius, sun_mu);
        let multiple = multiple_scattering(sample_radius, sun_mu);
        let source = air.rayleigh_scattering * (rayleigh * lit + multiple)
            + air.mie_scattering * (mie * lit + multiple);
        let step_transmittance = exp(-air.extinction * step);
        radiance += throughput * integrate_step(source, air.extinction, step_transmittance);
        throughput *= step_transmittance;
    }
    return radiance;
}

fn sky_radiance_direction(uv: vec2<f32>) -> vec3<f32> {
    let azimuth = (uv.x - 0.5) * 2.0 * PI;
    let elevation = (0.5 - uv.y) * PI;
    return vec3<f32>(
        sin(azimuth) * cos(elevation),
        sin(elevation),
        -cos(azimuth) * cos(elevation),
    );
}

#ifdef SKY_RADIANCE_SQRT
const SKY_RADIANCE_SCALE: f32 = f32(#{SKY_RADIANCE_SQRT});
#endif

fn encode_sky_radiance(radiance: vec3<f32>) -> vec3<f32> {
#ifdef SKY_RADIANCE_SQRT
    return sqrt(radiance / SKY_RADIANCE_SCALE);
#else
    return radiance;
#endif
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    let view = sky_radiance_direction(in.uv);
    let radiance = sky_radiance(view, sky.toward_sun.xyz) * sky.illuminance.rgb;
    return vec4<f32>(encode_sky_radiance(max(radiance, vec3<f32>(0.0))), 1.0);
}
