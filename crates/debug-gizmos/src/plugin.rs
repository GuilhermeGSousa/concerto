use concerto_app::{
    Plugin,
    schedule_groups::{Extract, First, Render},
};
use concerto_ecs::IntoSystemConfig;
use concerto_render::{
    device::RenderDevice, layouts::CameraLayout, resources::RenderContext, sets::RenderSet,
};

use crate::{
    pipeline::GizmoPipeline,
    render::{clear_gizmos, extract_gizmos, render_gizmos},
    storage::{GizmoStorage, RenderGizmos},
};

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
        app.add_system(First, clear_gizmos);
        app.render_mut().insert_resource(RenderGizmos::default());
        app.add_render_system(Extract, extract_gizmos)
            .add_render_system(Render, render_gizmos.in_set(RenderSet::Overlay));
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

        app.render_mut().insert_resource(pipeline);
    }
}
