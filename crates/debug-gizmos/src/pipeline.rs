use concerto_ecs::resource::Resource;
use concerto_render::layouts::CameraLayout;

use crate::vertex::GizmoVertex;

/// WGSL source for the gizmo line shader (per-vertex colour, camera at group 0).
const GIZMO_SHADER: &str = include_str!("shaders/gizmo.wgsl");

/// Holds the render pipeline used to draw gizmo lines.
///
/// The pipeline binds the camera uniform at `@group(0)` (reusing the render
/// crate's [`CameraLayout`] so the bind group is compatible) and draws a
/// `line_list` of [`GizmoVertex`] with alpha blending and no depth testing, so
/// gizmos always render on top of the scene.
#[derive(Resource)]
pub struct GizmoPipeline {
    pub pipeline: wgpu::RenderPipeline,
}

impl GizmoPipeline {
    pub fn new(
        device: &wgpu::Device,
        camera_layout: &CameraLayout,
        surface_format: wgpu::TextureFormat,
    ) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Gizmo Shader"),
            source: wgpu::ShaderSource::Wgsl(GIZMO_SHADER.into()),
        });

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("Gizmo Pipeline Layout"),
            bind_group_layouts: &[&camera_layout.camera_layout],
            push_constant_ranges: &[],
        });

        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("Gizmo Pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                buffers: &[GizmoVertex::describe()],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                targets: &[Some(wgpu::ColorTargetState {
                    format: surface_format,
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::LineList,
                strip_index_format: None,
                front_face: wgpu::FrontFace::Ccw,
                cull_mode: None,
                polygon_mode: wgpu::PolygonMode::Fill,
                unclipped_depth: false,
                conservative: false,
            },
            depth_stencil: None,
            multisample: wgpu::MultisampleState {
                count: 1,
                mask: !0,
                alpha_to_coverage_enabled: false,
            },
            multiview: None,
            cache: None,
        });

        Self { pipeline }
    }
}
