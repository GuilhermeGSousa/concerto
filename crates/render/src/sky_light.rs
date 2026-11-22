use std::f32::consts::{FRAC_PI_4, PI, TAU};

use concerto_color::LinearRgba;
use concerto_ecs::{component::Component, resource::Resource};
use glam::Vec3;

use crate::{capabilities::RenderCapabilities, components::render_entity::SyncWithRenderWorld};

/// One radiance sample over the sphere, for building spherical harmonics.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RadianceSample {
    pub direction: Vec3,
    pub radiance: Vec3,
    pub solid_angle: f32,
}

/// Irradiance as spherical harmonics up to band 2, one RGB coefficient per basis function.
#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub struct SphericalHarmonics {
    pub coefficients: [Vec3; 9],
}

impl SphericalHarmonics {
    /// Projects radiance arriving from unit directions and stores the irradiance it casts.
    pub fn from_radiance(samples: &[RadianceSample]) -> Self {
        let mut coefficients = [Vec3::ZERO; 9];
        for sample in samples {
            let weighted = sample.radiance * sample.solid_angle;
            for (coefficient, weight) in coefficients.iter_mut().zip(basis(sample.direction)) {
                *coefficient += weighted * weight;
            }
        }
        for (coefficient, lobe) in coefficients.iter_mut().zip(COSINE_LOBE) {
            *coefficient *= lobe;
        }
        Self { coefficients }
    }

    /// The irradiance on a surface facing the unit vector `normal`.
    pub fn irradiance(&self, normal: Vec3) -> Vec3 {
        basis(normal)
            .iter()
            .zip(&self.coefficients)
            .fold(Vec3::ZERO, |sum, (weight, coefficient)| {
                sum + *coefficient * *weight
            })
    }
}

const COSINE_LOBE: [f32; 9] = [
    PI,
    TAU / 3.0,
    TAU / 3.0,
    TAU / 3.0,
    FRAC_PI_4,
    FRAC_PI_4,
    FRAC_PI_4,
    FRAC_PI_4,
    FRAC_PI_4,
];

fn basis(direction: Vec3) -> [f32; 9] {
    let Vec3 { x, y, z } = direction;
    [
        0.282_094_8,
        0.488_602_5 * y,
        0.488_602_5 * z,
        0.488_602_5 * x,
        1.092_548_4 * x * y,
        1.092_548_4 * y * z,
        0.315_391_6 * (3.0 * z * z - 1.0),
        1.092_548_4 * x * z,
        0.546_274_2 * (x * x - y * y),
    ]
}

/// Light a camera's sky casts on surfaces, kept up to date by the sky; not saved.
#[derive(Component, Clone, Copy, Debug, PartialEq)]
#[require(SyncWithRenderWorld)]
pub struct CameraSkyLight {
    pub irradiance: SphericalHarmonics,
    pub night_color: LinearRgba,
    pub night_ambient: LinearRgba,
}

/// A factor multiplied into a light's color, kept up to date by systems such as the sky; not saved.
#[derive(Component, Clone, Copy, Debug, PartialEq)]
#[require(SyncWithRenderWorld)]
pub struct LightTint(pub LinearRgba);

/// The largest radiance an `Rgba8Unorm` [`SkyRadianceMap`] holds.
pub const SKY_RADIANCE_SCALE: u32 = 16;

/// Render world: sky radiance per world direction `d` at `u = atan2(d.x, -d.z) / 2π + 0.5`, `v = 0.5 - asin(d.y) / π`, as `Rgba16Float` or, without float targets, `Rgba8Unorm` holding `sqrt(radiance / SKY_RADIANCE_SCALE)`; its mip levels, where it has them, are the blur rough surfaces reflect.
#[derive(Resource)]
pub struct SkyRadianceMap {
    pub view: wgpu::TextureView,
    pub sampler: wgpu::Sampler,
    pub format: wgpu::TextureFormat,
}

impl SkyRadianceMap {
    /// The map's format on a GPU with `capabilities`.
    pub fn format_for(capabilities: &RenderCapabilities) -> wgpu::TextureFormat {
        if capabilities.float_render_targets {
            wgpu::TextureFormat::Rgba16Float
        } else {
            wgpu::TextureFormat::Rgba8Unorm
        }
    }

    /// Views `texture` as the map, wrapping in azimuth and clamped at the poles.
    pub fn new(device: &wgpu::Device, texture: &wgpu::Texture) -> Self {
        Self {
            view: texture.create_view(&wgpu::TextureViewDescriptor::default()),
            sampler: device.create_sampler(&wgpu::SamplerDescriptor {
                label: Some("sky_radiance_sampler"),
                address_mode_u: wgpu::AddressMode::Repeat,
                address_mode_v: wgpu::AddressMode::ClampToEdge,
                mag_filter: wgpu::FilterMode::Linear,
                min_filter: wgpu::FilterMode::Linear,
                ..Default::default()
            }),
            format: texture.format(),
        }
    }

    /// A 1×1 black map, the sky of a renderer that draws none.
    pub fn black(device: &wgpu::Device, capabilities: &RenderCapabilities) -> Self {
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("sky_radiance_black"),
            size: wgpu::Extent3d {
                width: 1,
                height: 1,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: Self::format_for(capabilities),
            usage: wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        Self::new(device, &texture)
    }
}

#[cfg(test)]
mod tests {
    use std::f32::consts::{PI, TAU};

    use glam::Vec3;
    use proptest::prelude::*;

    use super::{basis, RadianceSample, SphericalHarmonics};

    const COSINE_LOBE: [f32; 9] = [
        PI,
        TAU / 3.0,
        TAU / 3.0,
        TAU / 3.0,
        PI / 4.0,
        PI / 4.0,
        PI / 4.0,
        PI / 4.0,
        PI / 4.0,
    ];

    fn sphere(radiance: impl Fn(Vec3) -> Vec3) -> Vec<RadianceSample> {
        let count = 4096;
        let golden = std::f64::consts::PI * (3.0 - 5f64.sqrt());
        (0..count)
            .map(|index| {
                let y = 1.0 - (index as f64 + 0.5) * 2.0 / count as f64;
                let ring = (1.0 - y * y).sqrt();
                let azimuth = golden * index as f64;
                let direction = Vec3::new(
                    (ring * azimuth.cos()) as f32,
                    y as f32,
                    (ring * azimuth.sin()) as f32,
                );
                RadianceSample {
                    direction,
                    radiance: radiance(direction),
                    solid_angle: 4.0 * PI / count as f32,
                }
            })
            .collect()
    }

    fn near(actual: Vec3, expected: Vec3, tolerance: f32) -> bool {
        (actual - expected).abs().max_element() <= tolerance
    }

    proptest! {
        #[test]
        fn spherical_harmonics_reproduce_irradiance(
            constant in prop::array::uniform3(0.0f32..10.0),
            field in prop::array::uniform9(prop::array::uniform3(-1.0f32..1.0)),
            height in -1.0f32..=1.0,
            azimuth in 0.0f32..TAU,
        ) {
            let ring = (1.0 - height * height).max(0.0).sqrt();
            let normal = Vec3::new(ring * azimuth.cos(), height, ring * azimuth.sin());

            let constant = Vec3::from_array(constant);
            let uniform = SphericalHarmonics::from_radiance(&sphere(|_| constant));
            let irradiance = uniform.irradiance(normal);
            prop_assert!(near(irradiance, PI * constant, 1e-2), "{irradiance} != {}", PI * constant);

            let field = field.map(Vec3::from_array);
            let radiance = |direction: Vec3| {
                basis(direction)
                    .iter()
                    .zip(&field)
                    .fold(Vec3::ZERO, |sum, (weight, coefficient)| sum + *coefficient * *weight)
            };
            let analytic = basis(normal)
                .iter()
                .zip(&field)
                .zip(COSINE_LOBE)
                .fold(Vec3::ZERO, |sum, ((weight, coefficient), lobe)| {
                    sum + *coefficient * *weight * lobe
                });
            let projected = SphericalHarmonics::from_radiance(&sphere(radiance)).irradiance(normal);
            prop_assert!(near(projected, analytic, 1e-2), "{projected} != {analytic}");
        }
    }
}
