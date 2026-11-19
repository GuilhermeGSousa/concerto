use concerto_ecs::resource::Resource;

use crate::{
    capabilities::RenderCapabilities, components::camera::PixelRect, shader::create_shader_module,
};

#[derive(Resource)]
pub(crate) struct ViewportClearPipeline {
    pipeline: wgpu::RenderPipeline,
}

impl ViewportClearPipeline {
    pub(crate) fn new(
        device: &wgpu::Device,
        capabilities: &RenderCapabilities,
        format: wgpu::TextureFormat,
    ) -> Self {
        let module = create_shader_module(
            device,
            "Viewport Clear",
            include_str!("shaders/viewport_clear.wgsl"),
            &capabilities.shader_defs(),
        );
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("Viewport Clear Pipeline Layout"),
            bind_group_layouts: &[],
            immediate_size: 0,
        });
        let constant = wgpu::BlendComponent {
            src_factor: wgpu::BlendFactor::Constant,
            dst_factor: wgpu::BlendFactor::Zero,
            operation: wgpu::BlendOperation::Add,
        };
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("Viewport Clear Pipeline"),
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
                    blend: Some(wgpu::BlendState {
                        color: constant,
                        alpha: constant,
                    }),
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
        Self { pipeline }
    }

    pub(crate) fn fill(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        view: &wgpu::TextureView,
        rect: PixelRect,
        color: wgpu::Color,
    ) {
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            multiview_mask: None,
            label: Some("Viewport Clear Pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                depth_slice: None,
                view,
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
        rect.apply(&mut pass);
        pass.set_pipeline(&self.pipeline);
        pass.set_blend_constant(color);
        pass.draw(0..3, 0..1);
    }
}
