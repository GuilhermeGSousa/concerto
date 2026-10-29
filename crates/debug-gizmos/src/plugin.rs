use concerto_app::{
    Plugin,
    edit_time::AtEditTime,
    schedule_groups::{Extract, First, LateUpdate, Render},
};
use concerto_ecs::IntoSystemConfig;
use concerto_foundation::transform::systems::propagate_global_transforms;
use concerto_render::{
    device::RenderDevice, layouts::CameraLayout, resources::RenderContext, sets::RenderSet,
};

use crate::{
    pipeline::GizmoPipeline,
    render::{clear_gizmos, extract_gizmos, render_gizmos},
    skeleton::{SkeletonGizmo, draw_skeleton_gizmos},
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
        app.add_system(First, clear_gizmos.at_edit_time());
        app.register_type::<SkeletonGizmo>().add_system(
            LateUpdate,
            draw_skeleton_gizmos
                .after(propagate_global_transforms)
                .at_edit_time(),
        );
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

#[cfg(test)]
mod tests {
    use std::any::TypeId;

    use concerto_app::App;

    use super::DebugGizmosPlugin;
    use crate::SkeletonGizmo;

    #[test]
    fn the_skeleton_gizmo_is_registered_for_the_inspector() {
        let mut app = App::new();
        app.register_plugin(DebugGizmosPlugin);

        let registry = app.main_mut().world_mut().type_registry();
        assert!(registry.read().get(TypeId::of::<SkeletonGizmo>()).is_some());
    }
}
