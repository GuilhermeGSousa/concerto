//! Camera control as a stack.
//!
//! One [`MainCamera`] entity owns the window's render resources for the whole
//! run, unless the [`CameraDirector`] is built without one. Everything that wants to
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
//! cameras.get_entity(shot).unwrap().set_enabled(true);   // takes over
//! cameras.get_entity(shot).unwrap().set_enabled(false);  // hands control back
//! ```

pub mod director;
pub mod main_camera;
pub mod virtual_camera;

use concerto_app::{
    App, Plugin,
    edit_time::AtEditTime,
    schedule_groups::{Startup, Update},
};
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
        app.register_type::<VirtualCamera>();

        app.add_system(Startup, main_camera::spawn_main_camera);

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
    use concerto_app::{App, main_schedule::MainSchedulePlugin, plugins::TimePlugin};

    use super::{CameraDirector, CameraDirectorPlugin, MainCamera};

    #[test]
    fn the_plugin_spawns_one_main_camera_unless_the_game_turned_it_off_first() {
        for spawns in [true, false] {
            let mut app = App::new();
            app.register_plugin(MainSchedulePlugin)
                .register_plugin(TimePlugin);
            if !spawns {
                app.insert_resource(CameraDirector::default().with_main_camera(false));
            }
            app.register_plugin(CameraDirectorPlugin);
            app.finish_plugin_build();

            app.update();

            let world = app.main_mut().world_mut();
            let main_cameras: Vec<_> = world
                .query::<concerto_ecs::Entity, concerto_ecs::With<MainCamera>>()
                .iter(world)
                .collect();
            assert_eq!(main_cameras.len(), usize::from(spawns), "spawns {spawns}");
            assert_eq!(
                app.get_resource::<CameraDirector>().unwrap().main_camera(),
                main_cameras.first().copied(),
                "spawns {spawns}"
            );
        }
    }
}
