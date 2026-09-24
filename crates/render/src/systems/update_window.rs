use std::ops::Deref;

use concerto_app::extractor::Extracted;
use concerto_ecs::{
    events::event_reader::EventReader,
    query::{change_detection::DetectChanges, Query},
    resource::{Res, ResMut},
};
use concerto_window::{plugin::Window, winit_events::WindowEvent};

use crate::{
    components::camera::RenderCamera,
    device::RenderDevice,
    render_asset::{render_texture::RenderTexture, render_window::RenderWindow},
    resources::RenderContext,
};

pub(crate) fn request_window_resize(
    mut window_events: EventReader<WindowEvent>,
    mut window: ResMut<Window>,
) {
    for event in window_events.read() {
        if let winit::event::WindowEvent::Resized(_) = event.deref() {
            window.mark_changed();
        }
    }
}

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

    let Some(surface) = context.surface.clone() else {
        return;
    };

    let size = render_window.size();
    if size != (context.surface_config.width, context.surface_config.height) {
        context.surface_config.width = size.0;
        context.surface_config.height = size.1;
        surface.configure(&device, &context.surface_config);

        for mut render_camera in render_cameras.iter() {
            if render_camera.render_target.is_some() {
                continue; // RTT camera: fixed resolution, depth stays in sync with RTT
            }
            render_camera.depth_texture =
                RenderTexture::create_depth_texture(&device, size.0, size.1, "depth_texture");
        }
    }

    if let Ok(output) = surface.get_current_texture() {
        render_window.set_swapchain_texture(output);
    }
}
