use concerto_app::{schedule_groups::Render, Plugin};
use concerto_ecs::{IntoSystemConfig, Resource};
use concerto_foundation::transform::GlobalTransformRaw;
use concerto_mesh::Vertex;
use wgpu::{
    BindGroupLayoutDescriptor, BindGroupLayoutEntry, BindingType, DepthBiasState,
    DepthStencilState, MultisampleState, PipelineCompilationOptions, PipelineLayoutDescriptor,
    PrimitiveState, RenderPipelineDescriptor, ShaderModuleDescriptor, ShaderStages, StencilState,
    TextureFormat,
};

use crate::{
    assets::vertex::VertexBufferLayout,
    components::shadows::{render_shadow_maps, update_shadow_view_proj},
    device::RenderDevice,
    layouts::SkeletonLayout,
    sets::RenderSet,
};

#[derive(Resource)]
pub(crate) struct ShadowPipeline {
    pub(crate) pipeline: wgpu::RenderPipeline,
    pub(crate) bind_group_layout: wgpu::BindGroupLayout,
}

/// Renders shadow maps in [`RenderSet::Shadows`].
///
/// # Panics
///
/// Panics during [`finish`](Plugin::finish) if [`RenderPlugin`](crate::plugin::RenderPlugin)
/// was not registered before it.
pub struct ShadowPipelinePlugin;

impl Plugin for ShadowPipelinePlugin {
    fn build(&self, app: &mut concerto_app::App) {
        app.render_mut().add_system(
            Render,
            render_shadow_maps
                .in_set(RenderSet::Shadows)
                .after(update_shadow_view_proj),
        );
    }

    fn finish(&self, app: &mut concerto_app::App) {
        let device = app
            .render()
            .get_resource::<RenderDevice>()
            .expect("RenderDevice not found; register RenderPlugin before MaterialPlugin");

        let vs_module = device.create_shader_module(ShaderModuleDescriptor {
            label: Some("Shadows VS"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shaders/shadow.wgsl").into()),
        });

        let light_view_bind_group_layout =
            device.create_bind_group_layout(&BindGroupLayoutDescriptor {
                label: Some("Shadow Bind Group Layout"),
                entries: &[BindGroupLayoutEntry {
                    binding: 0,
                    visibility: ShaderStages::VERTEX,
                    ty: BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                }],
            });

        let skeleton_layout = app
            .render()
            .get_resource::<SkeletonLayout>()
            .expect("SkeletonLayout not found: make sure the RenderPlugin is registered.");

        let pipeline_layout = device.create_pipeline_layout(&PipelineLayoutDescriptor {
            label: Some("Shadow Pipeline Layout"),
            bind_group_layouts: &[&light_view_bind_group_layout, skeleton_layout],
            push_constant_ranges: &[],
        });

        let pipeline = device.create_render_pipeline(&RenderPipelineDescriptor {
            label: Some("Shadow Pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &vs_module,
                entry_point: Some("vs_main"),
                buffers: &[Vertex::describe(), GlobalTransformRaw::describe()],
                compilation_options: PipelineCompilationOptions::default(),
            },
            fragment: None,
            primitive: PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                strip_index_format: None,
                front_face: wgpu::FrontFace::Ccw,
                cull_mode: Some(wgpu::Face::Back),
                unclipped_depth: false,
                polygon_mode: wgpu::PolygonMode::Fill,
                conservative: false,
            },
            depth_stencil: Some(DepthStencilState {
                format: TextureFormat::Depth32Float,
                depth_write_enabled: true,
                depth_compare: wgpu::CompareFunction::Less,
                stencil: StencilState::default(),
                bias: DepthBiasState {
                    constant: 0,
                    slope_scale: 1.0,
                    clamp: 100.0,
                },
            }),
            multisample: MultisampleState {
                count: 1,
                mask: !0,
                alpha_to_coverage_enabled: false,
            },
            multiview: None,
            cache: None,
        });

        app.render_mut().insert_resource(ShadowPipeline {
            pipeline,
            bind_group_layout: light_view_bind_group_layout,
        });
    }
}
