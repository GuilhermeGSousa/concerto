use std::collections::HashMap;

use concerto_ecs::resource::Resource;
use concerto_render::layouts::CameraLayout;

use crate::vertex::{GizmoVertex, WideGizmoVertex};

/// WGSL source for the gizmo line shader (per-vertex colour, camera at group 0).
const GIZMO_SHADER: &str = include_str!("shaders/gizmo.wgsl");
const WIDE_SHADER: &str = include_str!("shaders/wide.wgsl");

/// Holds the render pipeline used to draw gizmo lines.
///
/// The pipeline binds the camera uniform at `@group(0)` (reusing the render
/// crate's [`CameraLayout`] so the bind group is compatible) and draws a
/// `line_list` of [`GizmoVertex`] with alpha blending and no depth testing, so
/// gizmos always render on top of the scene.
#[derive(Resource)]
pub struct GizmoPipeline {
    /// Layout of the per-camera target size the wide pipeline reads at group 1.
    pub viewport_layout: wgpu::BindGroupLayout,
    shader: wgpu::ShaderModule,
    wide_shader: wgpu::ShaderModule,
    layout: wgpu::PipelineLayout,
    wide_layout: wgpu::PipelineLayout,
    format: wgpu::TextureFormat,
    by_samples: HashMap<u32, GizmoPipelines>,
}

/// The gizmo pipelines for targets of one sample count.
pub struct GizmoPipelines {
    pub lines: wgpu::RenderPipeline,
    /// Draws segments wider than a pixel as screen-space quads.
    pub wide: wgpu::RenderPipeline,
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
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("Gizmo Pipeline Layout"),
            bind_group_layouts: &[Some(&camera_layout.camera_layout)],
            immediate_size: 0,
        });
        let wide_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Wide Gizmo Shader"),
            source: wgpu::ShaderSource::Wgsl(WIDE_SHADER.into()),
        });
        let viewport_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Gizmo Viewport Layout"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });
        let wide_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("Wide Gizmo Pipeline Layout"),
            bind_group_layouts: &[Some(&camera_layout.camera_layout), Some(&viewport_layout)],
            immediate_size: 0,
        });
        let mut pipeline = Self {
            viewport_layout,
            shader,
            wide_shader,
            layout,
            wide_layout,
            format: surface_format,
            by_samples: HashMap::new(),
        };
        pipeline.specialize(device, 1);
        pipeline
    }

    /// The pipelines for targets of `samples` samples per pixel, once built.
    pub fn get(&self, samples: u32) -> Option<&GizmoPipelines> {
        self.by_samples.get(&samples)
    }

    /// Builds the pipelines for targets of `samples` samples per pixel, unless they exist.
    pub fn specialize(&mut self, device: &wgpu::Device, samples: u32) {
        if self.by_samples.contains_key(&samples) {
            return;
        }
        let target = [Some(wgpu::ColorTargetState {
            format: self.format,
            blend: Some(wgpu::BlendState::ALPHA_BLENDING),
            write_mask: wgpu::ColorWrites::ALL,
        })];
        let multisample = wgpu::MultisampleState {
            count: samples,
            ..Default::default()
        };
        let lines = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("Gizmo Pipeline"),
            layout: Some(&self.layout),
            vertex: wgpu::VertexState {
                module: &self.shader,
                entry_point: Some("vs_main"),
                buffers: &[Some(GizmoVertex::describe())],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &self.shader,
                entry_point: Some("fs_main"),
                targets: &target,
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
            multisample,
            multiview_mask: None,
            cache: None,
        });
        let wide = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("Wide Gizmo Pipeline"),
            layout: Some(&self.wide_layout),
            vertex: wgpu::VertexState {
                module: &self.wide_shader,
                entry_point: Some("vs_main"),
                buffers: &[Some(WideGizmoVertex::describe())],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &self.wide_shader,
                entry_point: Some("fs_main"),
                targets: &target,
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                cull_mode: None,
                ..Default::default()
            },
            depth_stencil: None,
            multisample,
            multiview_mask: None,
            cache: None,
        });
        self.by_samples
            .insert(samples, GizmoPipelines { lines, wide });
    }
}
