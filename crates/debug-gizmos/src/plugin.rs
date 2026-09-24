use concerto_app::{Plugin, schedule_groups::Render};
use concerto_render::{device::RenderDevice, layouts::CameraLayout, resources::RenderContext};

use crate::{pipeline::GizmoPipeline, render::render_gizmos, storage::GizmoStorage};

/// Registers immediate-mode debug gizmos.
///
/// After adding this plugin, any system can request a
/// [`DebugGizmos`](crate::gizmos::DebugGizmos) parameter and draw lines,
/// spheres, cuboids, and other shapes for the current frame.
///
/// Must be registered *after* the render plugin: it reads GPU resources such as
/// [`RenderDevice`] and [`CameraLayout`] during [`Plugin::finish`].
pub struct DebugGizmosPlugin;

impl Plugin for DebugGizmosPlugin {
    fn build(&self, app: &mut concerto_app::App) {
        app.insert_resource(GizmoStorage::default());
        app.add_system(Render, render_gizmos);
    }

    fn finish(&self, app: &mut concerto_app::App) {
        let surface_format = app
            .render()
            .get_resource::<RenderContext>()
            .expect("RenderContext not found; register RenderPlugin before DebugGizmosPlugin")
            .surface_config
            .format;

        let camera_layout = app
            .render()
            .get_resource::<CameraLayout>()
            .expect("CameraLayout not found; register RenderPlugin before DebugGizmosPlugin");

        let device = app
            .render()
            .get_resource::<RenderDevice>()
            .expect("RenderDevice not found; register RenderPlugin before DebugGizmosPlugin");

        let pipeline = GizmoPipeline::new(device, camera_layout, surface_format);

        app.insert_resource(pipeline);
    }
}
