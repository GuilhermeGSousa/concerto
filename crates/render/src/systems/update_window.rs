use concerto_app::extractor::Extracted;
use concerto_ecs::{
    query::change_detection::DetectChanges,
    resource::{Res, ResMut},
};
use concerto_window::plugin::Window;

use crate::{
    device::RenderDevice,
    draw_target::DrawTarget,
    render_asset::{render_texture::RenderTexture, render_window::RenderWindow},
    resources::RenderContext,
};

pub(crate) fn extract_window(
    window: Extracted<Res<Window>>,
    mut render_window: ResMut<RenderWindow>,
) {
    render_window.set_size(window.size());
}

pub(crate) fn update_render_window(
    mut render_window: ResMut<RenderWindow>,
    mut context: ResMut<RenderContext>,
    device: Res<RenderDevice>,
) {
    if !render_window.is_changed() {
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
            }
            if let wgpu::CurrentSurfaceTexture::Success(output)
            | wgpu::CurrentSurfaceTexture::Suboptimal(output) = surface.get_current_texture()
            {
                render_window.set_swapchain_texture(output);
            }
        }
        DrawTarget::Texture(shared) => {
            if resized || render_window.get_view().is_none() {
                context.surface_config.width = size.0;
                context.surface_config.height = size.1;
                let texture = RenderTexture::camera_target(
                    &device,
                    context.surface_config.format,
                    size.0,
                    size.1,
                );
                render_window.set_offscreen(texture.view.clone());
                shared.publish(texture);
            }
        }
        DrawTarget::None => {}
    }
}
