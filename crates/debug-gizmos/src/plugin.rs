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
    skeleton::draw_skeleton_gizmos,
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
        app.add_system(
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
    use concerto_app::{App, schedule_groups::LateUpdate};
    use concerto_ecs::{
        component::scene::SceneEntityRef,
        entity::hierarchy::ChildOf,
        system::{executor::single_thread::SingleThreadedExecutor, schedule::Schedules},
    };
    use concerto_foundation::transform::GlobalTransform;
    use concerto_mesh::{SkeletonComponent, SkeletonGizmo};
    use glam::{Mat4, Vec3};

    use super::DebugGizmosPlugin;
    use crate::GizmoStorage;

    #[test]
    fn the_plugin_draws_skeleton_gizmos_in_late_update() {
        let mut app = App::new();
        app.register_plugin(DebugGizmosPlugin);
        let world = app.main_mut().world_mut();
        let hip = world.spawn(GlobalTransform::new(Mat4::from_translation(Vec3::Y)));
        let knee = world.spawn((
            GlobalTransform::new(Mat4::from_translation(Vec3::Y * 0.5)),
            ChildOf::new(hip),
        ));
        world.spawn((
            SkeletonComponent::new(
                Default::default(),
                vec![SceneEntityRef::Entity(hip), SceneEntityRef::Entity(knee)],
                None,
            ),
            SkeletonGizmo::default(),
        ));
        let schedules = world.remove_resource::<Schedules>().unwrap();
        let compiled = schedules.compile::<SingleThreadedExecutor>(world);
        world.insert_resource(compiled);

        world.run_schedule(LateUpdate);

        let storage = world.get_resource::<GizmoStorage>().unwrap();
        assert!(!storage.vertices.is_empty());
    }
}
