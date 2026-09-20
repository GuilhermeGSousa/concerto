//! Camera control as a stack.
//!
//! One [`MainCamera`] entity owns the window's render resources for the whole
//! run. Everything that wants to control the view spawns a [`VirtualCamera`] —
//! a pose with a priority — and the [`CameraDirector`] stacks them by priority.
//! Whatever is on top drives the main camera, blending on takeover.
//!
//! Having the component is being in the stack, and there is no way to
//! re-prioritise in place: a camera enters and leaves by the component being
//! added and removed. Whether an entry is *eligible* for control is the one
//! runtime toggle, and it lives on the director's stack entry rather than the
//! component, so flipping it takes effect immediately.
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
//! director.set_enabled(shot, true);   // takes over
//! director.set_enabled(shot, false);  // hands control back
//! ```

pub mod director;
pub mod main_camera;
pub mod virtual_camera;

use app::{
    App, Plugin,
    schedule_groups::{Startup, Update},
};

pub use director::CameraDirector;
pub use main_camera::MainCamera;
pub use virtual_camera::{BlendIn, CameraPose, Ease, Lens, VirtualCamera};

/// Registers the main camera and the virtual-camera director.
///
/// The director runs in `Update`, so nothing about the pose pipeline depends on
/// where this plugin sits in the registration order. Demotion is the exception:
/// it must beat `camera_added`, so leave this registered before `RenderPlugin`
/// or a stray window camera gets a frame of double rendering (and keeps its
/// `RenderCamera` afterwards).
pub struct CameraDirectorPlugin;

impl Plugin for CameraDirectorPlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(CameraDirector::default());
        app.register_scene_component::<VirtualCamera>();

        app.add_system(Startup, main_camera::spawn_main_camera);

        app.add_system(Update, director::drive_main_camera);
    }
}
