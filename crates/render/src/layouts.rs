use concerto_ecs::resource::Resource;
use derive_more::Deref;
use wgpu::BindGroupLayoutDescriptor;

use crate::{
    capabilities::RenderCapabilities,
    components::shadows::{PointShadowKind, ShadowMapKind},
};

/// Bind-group layout for the camera uniform (`@group(1) @binding(0)` in the
/// default material convention), the sky-radiance map (bindings 1 and 2) and
/// the camera's ambient occlusion (binding 3), white where it has none.
///
/// Exposed publicly so crates with their own render passes (e.g. debug gizmos)
/// can build a pipeline whose camera bind-group layout is *the same object*
/// that [`RenderCamera`](crate::components::camera::RenderCamera) bind groups
/// are created from, guaranteeing wgpu bind-group compatibility.
#[derive(Resource)]
pub struct CameraLayout {
    pub camera_layout: wgpu::BindGroupLayout,
}

impl CameraLayout {
    pub fn new(device: &wgpu::Device) -> Self {
        let camera_bind_group_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                entries: &[
                    wgpu::BindGroupLayoutEntry {
                        binding: 0,
                        visibility: wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Uniform,
                            has_dynamic_offset: false,
                            min_binding_size: None,
                        },
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 1,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Texture {
                            sample_type: wgpu::TextureSampleType::Float { filterable: true },
                            view_dimension: wgpu::TextureViewDimension::D2,
                            multisampled: false,
                        },
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 2,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 3,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Texture {
                            sample_type: wgpu::TextureSampleType::Float { filterable: true },
                            view_dimension: wgpu::TextureViewDimension::D2,
                            multisampled: false,
                        },
                        count: None,
                    },
                ],
                label: Some("camera_bind_group_layout"),
            });

        Self {
            camera_layout: camera_bind_group_layout,
        }
    }
}

#[derive(Resource, Deref)]
pub(crate) struct SkeletonLayout(pub(crate) wgpu::BindGroupLayout);

impl SkeletonLayout {
    pub fn new(device: &wgpu::Device) -> Self {
        let skeleton_layout = device.create_bind_group_layout(&BindGroupLayoutDescriptor {
            label: Some("skeleton_bind_group_layout"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: true,
                    min_binding_size: None,
                },
                count: None,
            }],
        });

        Self(skeleton_layout)
    }
}

/// Bind-group layout for the lights uniform and the shadow maps, `@group(2)` in the default material convention.
///
/// The group holds the lights uniform, both shadow-map arrays and the spot and
/// directional shadow view-projection array. Keeping them in one group lets
/// camera (1), lighting (2) and skeleton (3) fit the four bind groups wgpu
/// guarantees (`max_bind_groups`) without an elevated device limit. Lights and
/// shadow sampling change once per frame rather than per draw, so merging them
/// adds no per-camera invalidation.
#[derive(Resource, Deref)]
pub struct LightingLayout(pub(crate) wgpu::BindGroupLayout);

impl LightingLayout {
    pub fn new(device: &wgpu::Device, capabilities: &RenderCapabilities) -> Self {
        let layout = device.create_bind_group_layout(&BindGroupLayoutDescriptor {
            label: Some("lighting_bind_group_layout"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Depth,
                        view_dimension: wgpu::TextureViewDimension::D2Array,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Comparison),
                    count: None,
                },
                // Point-light shadow cube-map array: point lights are
                // omnidirectional and need a full cube per caster.
                wgpu::BindGroupLayoutEntry {
                    binding: 3,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Depth,
                        view_dimension: PointShadowKind::array_view_dimension(capabilities),
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 4,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Comparison),
                    count: None,
                },
                // Spot/directional shadow view-proj matrices, indexed by
                // `shadow_layer` in the shader — see `RenderShadowViewProjs`.
                wgpu::BindGroupLayoutEntry {
                    binding: 5,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
            ],
        });

        Self(layout)
    }
}
