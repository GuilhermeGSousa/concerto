use concerto_ecs::{
    component::Component,
    query::Query,
    reflect::{ReflectComponent, ReflectSceneComponent},
    resource::{Res, ResMut, Resource},
};
use concerto_reflect::{Reflect, ReflectDefault};

use crate::{
    capabilities::RenderCapabilities, components::camera::RenderCamera, device::RenderDevice,
    layouts::CameraLayout, render_asset::render_window::RenderWindow, resources::RenderContext,
    shader::create_shader_module,
};

/// The curve that maps a camera's scene light to the range its target can show.
///
/// Goes on the camera's entity. A camera without one uses [`Tonemapping::Aces`].
#[derive(Component, Clone, Copy, Debug, Default, PartialEq, Eq, Reflect)]
#[reflect(Component, Default, SceneComponent)]
pub enum Tonemapping {
    /// Scene light is clamped to the displayable range and shown as it is.
    None,
    /// Narkowicz's fit of the ACES filmic curve.
    #[default]
    Aces,
}

impl Tonemapping {
    pub(crate) fn shader_value(self) -> u32 {
        match self {
            Tonemapping::None => 0,
            Tonemapping::Aces => 1,
        }
    }

    /// The colour shown for linear scene light `color`, mirroring the tonemapping shader.
    pub fn apply(self, color: glam::Vec3) -> glam::Vec3 {
        let color = color.max(glam::Vec3::ZERO);
        match self {
            Tonemapping::None => color.min(glam::Vec3::ONE),
            Tonemapping::Aces => ((color * (2.51 * color + 0.03))
                / (color * (2.43 * color + 0.59) + 0.14))
                .clamp(glam::Vec3::ZERO, glam::Vec3::ONE),
        }
    }

    /// The scene light this curve shows as `shown`, the inverse of [`apply`](Self::apply) inside the displayable range.
    pub fn scene_light(self, shown: glam::Vec3) -> glam::Vec3 {
        let shown = shown.clamp(glam::Vec3::ZERO, glam::Vec3::ONE);
        match self {
            Tonemapping::None => shown,
            Tonemapping::Aces => {
                let a = 2.51 - 2.43 * shown;
                let b = 0.03 - 0.59 * shown;
                let c = -0.14 * shown;
                let discriminant = (b * b - 4.0 * a * c).max(glam::Vec3::ZERO);
                (-b + glam::Vec3::new(
                    discriminant.x.sqrt(),
                    discriminant.y.sqrt(),
                    discriminant.z.sqrt(),
                )) / (2.0 * a)
            }
        }
    }
}

/// How bright a camera's scene light is shown, in photographic stops.
///
/// Goes on the camera's entity. A camera without one shows it unscaled.
#[derive(Component, Clone, Copy, Debug, Default, PartialEq, Reflect)]
#[reflect(Component, Default, SceneComponent)]
pub struct Exposure {
    /// Each stop up doubles the scene's brightness before tonemapping, and each stop down halves it.
    pub stops: f32,
}

impl Exposure {
    /// An exposure of `stops` above or below the unscaled scene.
    pub fn new(stops: f32) -> Self {
        Self { stops }
    }

    /// The factor scene light is multiplied by before tonemapping.
    pub fn multiplier(&self) -> f32 {
        self.stops.exp2()
    }
}

#[derive(Resource)]
pub(crate) struct TonemappingPipeline {
    pipeline: wgpu::RenderPipeline,
    scene_layout: wgpu::BindGroupLayout,
    unadapted: wgpu::TextureView,
}

impl TonemappingPipeline {
    pub(crate) fn new(
        device: &wgpu::Device,
        capabilities: &RenderCapabilities,
        camera_layout: &CameraLayout,
        format: wgpu::TextureFormat,
    ) -> Self {
        let module = create_shader_module(
            device,
            "Tonemapping",
            include_str!("shaders/tonemapping.wgsl"),
            &capabilities.shader_defs(),
        );
        let texture = |binding| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Texture {
                sample_type: wgpu::TextureSampleType::Float { filterable: true },
                view_dimension: wgpu::TextureViewDimension::D2,
                multisampled: false,
            },
            count: None,
        };
        let scene_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Tonemapping Scene Layout"),
            entries: &[texture(0), texture(1)],
        });
        let unadapted = device
            .create_texture(&wgpu::TextureDescriptor {
                label: Some("Unadapted Exposure"),
                size: wgpu::Extent3d {
                    width: 1,
                    height: 1,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba8Unorm,
                usage: wgpu::TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            })
            .create_view(&wgpu::TextureViewDescriptor::default());
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("Tonemapping Pipeline Layout"),
            bind_group_layouts: &[Some(&camera_layout.camera_layout), Some(&scene_layout)],
            immediate_size: 0,
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("Tonemapping Pipeline"),
            layout: Some(&layout),
            vertex: wgpu::VertexState {
                module: &module,
                entry_point: Some("vs_main"),
                buffers: &[],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &module,
                entry_point: Some("fs_main"),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            }),
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });
        Self {
            pipeline,
            scene_layout,
            unadapted,
        }
    }
}

pub(crate) fn tonemap_cameras(
    pipeline: Res<TonemappingPipeline>,
    mut device: ResMut<RenderDevice>,
    render_cameras: Query<&RenderCamera>,
    render_window: Res<RenderWindow>,
    context: Res<RenderContext>,
) {
    for render_camera in render_cameras.iter() {
        let rect = render_camera.viewport_rect(render_camera.target_size(&context));
        if rect.is_empty() {
            continue;
        }
        let Some(target) = render_camera.output_view(&render_window) else {
            continue;
        };
        let scene = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Tonemapping Scene Bind Group"),
            layout: &pipeline.scene_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(
                        &render_camera.view_target.main_texture().view,
                    ),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(
                        render_camera
                            .adapted_exposure
                            .as_ref()
                            .unwrap_or(&pipeline.unadapted),
                    ),
                },
            ],
        });
        let mut pass = device
            .command_encoder()
            .begin_render_pass(&wgpu::RenderPassDescriptor {
                multiview_mask: None,
                label: Some("Tonemapping Pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    depth_slice: None,
                    view: target,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Load,
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                occlusion_query_set: None,
                timestamp_writes: None,
            });
        if render_camera.render_target.is_none() {
            rect.apply(&mut pass);
        }
        pass.set_pipeline(&pipeline.pipeline);
        pass.set_bind_group(0, &render_camera.camera_bind_group, &[]);
        pass.set_bind_group(1, &scene, &[]);
        pass.draw(0..3, 0..1);
    }
}

#[cfg(test)]
mod tests {
    use glam::Vec3;

    use super::{Exposure, Tonemapping};

    #[test]
    fn each_curve_maps_scene_light_into_the_displayable_range() {
        let light = Vec3::new(0.0, 0.5, 4.0);

        assert_eq!(Tonemapping::None.apply(light), Vec3::new(0.0, 0.5, 1.0));
        let aces = Tonemapping::Aces.apply(light);
        assert_eq!(aces.x, 0.0);
        assert!((aces.y - 0.6163).abs() < 1e-3, "{aces}");
        assert!((aces.z - 0.9735).abs() < 1e-3, "{aces}");

        for curve in [Tonemapping::None, Tonemapping::Aces] {
            for shown in [Vec3::ZERO, Vec3::new(0.008, 0.5, 0.9), Vec3::ONE] {
                let round_trip = curve.apply(curve.scene_light(shown));
                assert!(
                    (round_trip - shown).abs().max_element() < 1e-4,
                    "{curve:?} shows the light it gives for {shown} as {round_trip}"
                );
            }
        }

        assert_eq!(Exposure::default().multiplier(), 1.0);
        assert_eq!(Exposure::new(2.0).multiplier(), 4.0);
        assert_eq!(Exposure::new(-1.0).multiplier(), 0.5);
    }
}
