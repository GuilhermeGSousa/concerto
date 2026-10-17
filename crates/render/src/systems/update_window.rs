use concerto_app::extractor::Extracted;
use concerto_ecs::{
    query::{change_detection::DetectChanges, Query},
    resource::{Res, ResMut},
};
use concerto_window::plugin::Window;

use crate::{
    components::camera::{create_rtt, RenderCamera},
    device::RenderDevice,
    draw_target::DrawTarget,
    render_asset::{render_texture::RenderTexture, render_window::RenderWindow},
    resources::RenderContext,
};

pub(crate) fn extract_window(
    window: Extracted<Res<Window>>,
    mut render_window: ResMut<RenderWindow>,
) {
    // if !window.has_changed() {
    //     return;
    // }

    render_window.set_size(window.size());
}

pub(crate) fn update_render_window(
    mut render_window: ResMut<RenderWindow>,
    mut context: ResMut<RenderContext>,
    device: Res<RenderDevice>,
    render_cameras: Query<&mut RenderCamera>,
) {
    if !render_window.has_changed() {
        return;
    }

    let size = render_window.size();
    if size.0 == 0 || size.1 == 0 {
        return;
    }
    let resized = size != (context.surface_config.width, context.surface_config.height);

    match context.target.clone() {
        DrawTarget::Surface(surface) => {
            if resized {
                context.surface_config.width = size.0;
                context.surface_config.height = size.1;
                surface.configure(&device, &context.surface_config);
                resize_window_depth(&device, &render_cameras, size);
            }
            if let Ok(output) = surface.get_current_texture() {
                render_window.set_swapchain_texture(output);
            }
        }
        DrawTarget::Texture(shared) => {
            if resized || render_window.get_view().is_none() {
                context.surface_config.width = size.0;
                context.surface_config.height = size.1;
                let texture = create_rtt(&device, context.surface_config.format, size.0, size.1);
                render_window.set_offscreen(texture.view.clone());
                shared.publish(texture);
                resize_window_depth(&device, &render_cameras, size);
            }
        }
        DrawTarget::None => {}
    }
}

fn resize_window_depth(
    device: &RenderDevice,
    render_cameras: &Query<&mut RenderCamera>,
    size: (u32, u32),
) {
    for mut render_camera in render_cameras.iter() {
        if render_camera.render_target.is_some() {
            continue; // RTT camera: fixed resolution, depth stays in sync with RTT
        }
        render_camera.depth_texture =
            RenderTexture::create_depth_texture(device, size.0, size.1, "depth_texture");
    }
}
