use concerto_ecs::{CommandQueue, Component, ResMut};
use concerto_foundation::transform::Transform;
use concerto_render::components::{camera::Camera, render_entity::SyncWithRenderWorld};

use crate::{director::CameraDirector, virtual_camera::VirtualCamera};

/// Marks the one [`Camera`] that renders to the window.
///
/// Spawned once at startup and never despawned or re-parented: there is no
/// `camera_removed` counterpart to `camera_added`, so removing the `Camera`
/// component leaves its `RenderCamera` rendering forever, and the director
/// writes world-space poses straight to its `Transform`, which a parent would
/// reinterpret as local.
#[derive(Component)]
pub struct MainCamera;

/// Spawns the main camera plus a lowest-priority fallback virtual camera, so the
/// stack is never empty and the window always has a view.
pub(crate) fn spawn_main_camera(mut cmd: CommandQueue, mut director: ResMut<CameraDirector>) {
    let main_camera = cmd
        .spawn((
            MainCamera,
            SyncWithRenderWorld,
            Camera::default(),
            Transform::default(),
        ))
        .entity();
    director.set_main_camera(main_camera);

    cmd.spawn((VirtualCamera::new(i32::MIN), Transform::default()));
}
