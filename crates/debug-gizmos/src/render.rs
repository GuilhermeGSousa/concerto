use concerto_ecs::{
    query::Query,
    resource::{Res, ResMut},
};
use concerto_render::{
    components::camera::RenderCamera, device::RenderDevice,
    render_asset::render_window::RenderWindow, resources::RenderContext,
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
    mut pipeline: ResMut<GizmoPipeline>,
    render_cameras: Query<&RenderCamera>,
    render_window: Res<RenderWindow>,
    context: Res<RenderContext>,
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
            let rect = render_camera.viewport_rect(render_camera.target_size(&context));
            let size = [rect.width as f32, rect.height as f32, 0.0, 0.0];
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

    for render_camera in render_cameras.iter() {
        pipeline.specialize(&device, render_camera.samples());
    }
    let encoder = device.command_encoder();

    for (render_camera, viewport) in render_cameras.iter().zip(&viewports) {
        let rect = render_camera.viewport_rect(render_camera.target_size(&context));
        if rect.is_empty() {
            continue;
        }
        let Some(color_view) = render_camera.color_view(render_window.get_view()) else {
            continue;
        };
        let Some(pipelines) = pipeline.get(render_camera.samples()) else {
            continue;
        };

        let mut render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            multiview_mask: None,
            label: Some("Gizmo Render Pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                depth_slice: None,
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
        rect.apply(&mut render_pass);

        render_pass.set_bind_group(0, &render_camera.camera_bind_group, &[]);
        if let Some(lines) = &lines {
            render_pass.set_pipeline(&pipelines.lines);
            render_pass.set_vertex_buffer(0, lines.slice(..));
            render_pass.draw(0..line_count, 0..1);
        }
        if let (Some(wide), Some(viewport)) = (&wide, viewport) {
            render_pass.set_pipeline(&pipelines.wide);
            render_pass.set_bind_group(1, viewport, &[]);
            render_pass.set_vertex_buffer(0, wide.slice(..));
            render_pass.draw(0..wide_count, 0..1);
        }
    }
}
