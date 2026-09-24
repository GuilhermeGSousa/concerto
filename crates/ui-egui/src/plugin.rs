use concerto_app::{
    plugins::Plugin,
    schedule_groups::{LateRender, Render, Update},
};
use concerto_render::{device::RenderDevice, resources::RenderContext};

use crate::{
    input::handle_window_events,
    render::{begin_ui_frame, end_ui_frame},
    resources::UIRenderer,
};

pub struct UIPlugin;

impl Plugin for UIPlugin {
    fn build(&self, app: &mut concerto_app::App) {
        app.add_system(Update, handle_window_events);
        app.add_system(Render, begin_ui_frame);
        // TODO: Fix this
        app.add_system(LateRender, end_ui_frame);
    }

    fn finish(&self, app: &mut concerto_app::App) {
        let device = app
            .get_resource::<RenderDevice>()
            .expect("RenderContext resource not found");
        let context = app
            .get_resource::<RenderContext>()
            .expect("RenderContext resource not found");
        let window = app
            .get_resource::<concerto_window::plugin::Window>()
            .expect("Window resource not found");

        app.insert_resource(UIRenderer::new(
            &window.window_handle,
            &device,
            context.surface_config.format,
            None,
            1,
            true,
        ));
    }
}
