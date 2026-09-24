use concerto_ecs::{
    query::Query,
    resource::{Res, ResMut},
};
use concerto_render::{
    components::camera::RenderCamera, device::RenderDevice,
    render_asset::render_window::RenderWindow,
};
use wgpu::util::DeviceExt;

use crate::{pipeline::GizmoPipeline, storage::GizmoStorage};

/// Uploads all gizmos buffered this frame and draws them once per camera, then
/// clears the storage so the next frame starts empty (immediate mode).
///
/// Runs in [`Render`](concerto_app::schedule_groups::Render).
/// It records into the shared frame encoder after the material passes, so
/// gizmos are drawn on top of the scene.
pub(crate) fn render_gizmos(
    mut storage: ResMut<GizmoStorage>,
    mut device: ResMut<RenderDevice>,
    pipeline: Res<GizmoPipeline>,
    render_cameras: Query<&RenderCamera>,
    render_window: Res<RenderWindow>,
) {
    if storage.vertices.is_empty() {
        return;
    }

    let vertex_count = storage.vertices.len() as u32;
    let vertex_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("Gizmo Vertex Buffer"),
        contents: bytemuck::cast_slice(&storage.vertices),
        usage: wgpu::BufferUsages::VERTEX,
    });

    let encoder = device.command_encoder();

    for render_camera in render_cameras.iter() {
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

        render_pass.set_pipeline(&pipeline.pipeline);
        render_pass.set_bind_group(0, &render_camera.camera_bind_group, &[]);
        render_pass.set_vertex_buffer(0, vertex_buffer.slice(..));
        render_pass.draw(0..vertex_count, 0..1);
    }

    storage.clear();
}
