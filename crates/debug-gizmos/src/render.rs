use concerto_ecs::{
    query::Query,
    resource::{Res, ResMut},
};
use concerto_render::{
    components::camera::RenderCamera, device::RenderDevice,
    render_asset::render_window::RenderWindow,
};
use wgpu::util::DeviceExt;

use concerto_app::extractor::Extracted;

use crate::{
    pipeline::GizmoPipeline,
    storage::{GizmoStorage, RenderGizmos},
};

pub(crate) fn clear_gizmos(mut storage: ResMut<GizmoStorage>) {
    if !storage.is_empty() {
        storage.clear();
    }
}

pub(crate) fn extract_gizmos(main: Extracted<Res<GizmoStorage>>, mut render: ResMut<RenderGizmos>) {
    render.replace_with(&main);
}

/// Draws the extracted gizmos once per camera, over the scene that camera has already drawn.
pub(crate) fn render_gizmos(
    storage: Res<RenderGizmos>,
    mut device: ResMut<RenderDevice>,
    pipeline: Res<GizmoPipeline>,
    render_cameras: Query<&RenderCamera>,
    render_window: Res<RenderWindow>,
) {
    if storage.vertices.is_empty() && storage.wide.is_empty() {
        return;
    }

    let line_count = storage.vertices.len() as u32;
    let lines = (line_count > 0).then(|| {
        device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Gizmo Vertex Buffer"),
            contents: bytemuck::cast_slice(&storage.vertices),
            usage: wgpu::BufferUsages::VERTEX,
        })
    });
    let wide_count = storage.wide.len() as u32;
    let wide = (wide_count > 0).then(|| {
        device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Wide Gizmo Vertex Buffer"),
            contents: bytemuck::cast_slice(&storage.wide),
            usage: wgpu::BufferUsages::VERTEX,
        })
    });
    let viewports: Vec<Option<wgpu::BindGroup>> = render_cameras
        .iter()
        .map(|render_camera| {
            wide.as_ref()?;
            let target = render_camera.depth_texture().texture();
            let size = [target.width() as f32, target.height() as f32, 0.0, 0.0];
            let buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("Gizmo Viewport Buffer"),
                contents: bytemuck::cast_slice(&size),
                usage: wgpu::BufferUsages::UNIFORM,
            });
            Some(device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("Gizmo Viewport Bind Group"),
                layout: &pipeline.viewport_layout,
                entries: &[wgpu::BindGroupEntry {
                    binding: 0,
                    resource: buffer.as_entire_binding(),
                }],
            }))
        })
        .collect();

    let encoder = device.command_encoder();

    for (render_camera, viewport) in render_cameras.iter().zip(&viewports) {
        let swapchain_view = render_window.get_view();
        let color_view: &wgpu::TextureView = match &render_camera.render_target {
            Some(rt) => &rt.view,
            None => match swapchain_view {
                Some(v) => v,
                None => continue,
            },
        };

        let mut render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("Gizmo Render Pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: color_view,
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

        render_pass.set_bind_group(0, &render_camera.camera_bind_group, &[]);
        if let Some(lines) = &lines {
            render_pass.set_pipeline(&pipeline.pipeline);
            render_pass.set_vertex_buffer(0, lines.slice(..));
            render_pass.draw(0..line_count, 0..1);
        }
        if let (Some(wide), Some(viewport)) = (&wide, viewport) {
            render_pass.set_pipeline(&pipeline.wide_pipeline);
            render_pass.set_bind_group(1, viewport, &[]);
            render_pass.set_vertex_buffer(0, wide.slice(..));
            render_pass.draw(0..wide_count, 0..1);
        }
    }
}
