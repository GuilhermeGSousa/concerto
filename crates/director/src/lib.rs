//! Camera control as a stack.
//!
//! The level holds the real camera, marked [`MainCamera`]; the director spawns
//! none, and drives the one with the lowest entity index. Everything that wants to
//! control the view spawns a [`VirtualCamera`] — a pose with a priority — and
//! the [`CameraDirector`] stacks them by priority. Whatever is on top drives the
//! main camera, blending on takeover.
//!
//! Having the component is being in the stack: a camera enters and leaves by
//! the component being added and removed. `priority` and `enabled` are read from
//! the component, so editing either in place, through reflection or
//! [`VirtualCamera::set_enabled`], re-sorts the stack on the director's next run.
//!
//! ```ignore
//! // A player rig: the pivot follows the character, the child is the pose.
//! cmd.spawn((CameraPivot::default(), Transform::default(), EntityFollow { .. }))
//!     .add_child((
//!         VirtualCamera::new(10),
//!         Transform::from_translation(Vec3::NEG_Z * 10.0),
//!     ));
//!
//! // A cutscene camera that ships with the scene, dormant until its cue.
//! let shot = cmd
//!     .spawn((VirtualCamera::new(100).disabled(), Transform::default()))
//!     .entity();
//!
//! cameras.get_entity_mut(shot).unwrap().set_enabled(true);   // takes over
//! cameras.get_entity_mut(shot).unwrap().set_enabled(false);  // hands control back
//! ```

/// [`CameraDirector`], the stack of virtual cameras, and the systems that drive the main camera.
pub mod director;
/// [`MainCamera`], the marker of the level's camera.
pub mod main_camera;
/// [`VirtualCamera`] and the pose, lens and blend types it is made of.
pub mod virtual_camera;

use concerto_app::{App, Plugin, edit_time::AtEditTime, schedule_groups::Update};
use concerto_ecs::IntoSystemConfig;

pub use director::CameraDirector;
pub use main_camera::MainCamera;
pub use virtual_camera::{BlendIn, CameraPose, Ease, Lens, VirtualCamera};

/// Registers the main camera and the virtual-camera director.
///
/// The director runs in `Update`, so nothing about the pose pipeline depends on
/// where this plugin sits in the registration order. The stack sync runs before
/// the pose is driven, in an edit `App` as well.
pub struct CameraDirectorPlugin;

impl Plugin for CameraDirectorPlugin {
    fn build(&self, app: &mut App) {
        if app.get_resource::<CameraDirector>().is_none() {
            app.insert_resource(CameraDirector::default());
        }
        app.register_type::<VirtualCamera>()
            .register_type::<MainCamera>();

        app.add_system(
            Update,
            main_camera::adopt_main_camera.before(director::drive_main_camera),
        );
        app.add_system(Update, director::drive_main_camera);
        app.add_system(
            Update,
            director::sync_virtual_cameras
                .before(director::drive_main_camera)
                .at_edit_time(),
        );
    }
}

#[cfg(test)]
mod tests {
    use concerto_app::{
        App,
        main_schedule::MainSchedulePlugin,
        plugins::{TimePlugin, TransformPlugin},
    };
    use concerto_ecs::{Entity, With};
    use concerto_foundation::transform::{GlobalTransform, Transform};
    use glam::Vec3;

    use super::{CameraDirector, CameraDirectorPlugin, MainCamera, VirtualCamera};

    fn global(app: &mut App, entity: Entity) -> Vec3 {
        app.main_mut()
            .world_mut()
            .get_component_for_entity::<GlobalTransform>(entity)
            .unwrap()
            .translation()
    }

    fn settle(app: &mut App) {
        for _ in 0..3 {
            app.update();
        }
    }

    #[test]
    fn the_director_drives_the_level_main_camera() {
        let mut app = App::new();
        app.register_plugin(MainSchedulePlugin)
            .register_plugin(TimePlugin)
            .register_plugin(TransformPlugin)
            .register_plugin(CameraDirectorPlugin);
        app.finish_plugin_build();
        app.update();
        let world = app.main_mut().world_mut();
        assert_eq!(
            world
                .query::<Entity, With<MainCamera>>()
                .iter(world)
                .count(),
            0,
            "the director spawns no camera"
        );

        let parent = world.spawn(Transform::from_translation(Vec3::X * 10.0));
        let driven = world.spawn((MainCamera, Transform::from_translation(Vec3::Z * 5.0)));
        world.entity_mut(parent).add_child(driven);
        let other = world.spawn((MainCamera, Transform::from_translation(Vec3::Y)));
        settle(&mut app);
        assert_eq!(
            app.get_resource::<CameraDirector>().unwrap().main_camera(),
            Some(driven),
            "the lowest entity index is driven"
        );
        assert_eq!(
            global(&mut app, driven),
            Vec3::new(10.0, 0.0, 5.0),
            "with no live virtual camera the authored pose stays"
        );

        let shot = app.main_mut().world_mut().spawn((
            VirtualCamera::new(0),
            Transform::from_translation(Vec3::new(1.0, 2.0, 3.0)),
        ));
        settle(&mut app);
        let taken = global(&mut app, driven);
        assert!(
            taken.distance(Vec3::new(1.0, 2.0, 3.0)) < 1e-4,
            "the parented camera reaches the virtual camera's world pose: {taken}"
        );
        assert_eq!(global(&mut app, other), Vec3::Y);

        app.main_mut().world_mut().despawn(shot);
        settle(&mut app);
        let held = global(&mut app, driven);
        assert!(
            held.distance(Vec3::new(1.0, 2.0, 3.0)) < 1e-4,
            "the camera stays where the last live camera left it: {held}"
        );
    }
}
