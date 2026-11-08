use concerto_ecs::{CommandQueue, Component, ResMut};
use concerto_foundation::transform::Transform;
use concerto_render::components::camera::Camera;

use crate::{director::CameraDirector, virtual_camera::VirtualCamera};

/// Marks the camera the director drives.
///
/// Spawned once at startup unless the [`CameraDirector`] is built without one, and never
/// despawned or re-parented: there is no `camera_removed` counterpart to
/// `camera_added`, so removing the `Camera` component leaves its `RenderCamera`
/// rendering forever, and the director writes world-space poses straight to its
/// `Transform`, which a parent would reinterpret as local.
#[derive(Component)]
pub struct MainCamera;

/// Spawns the main camera plus a lowest-priority fallback virtual camera, so the
/// stack is never empty and the window always has a view, unless the director is built without a main camera.
pub(crate) fn spawn_main_camera(mut cmd: CommandQueue, mut director: ResMut<CameraDirector>) {
    if !director.spawns_main_camera() {
        return;
    }

    let main_camera = cmd
        .spawn((MainCamera, Camera::default(), Transform::default()))
        .entity();
    director.set_main_camera(main_camera);

    cmd.spawn((VirtualCamera::new(i32::MIN), Transform::default()));
}

#[cfg(test)]
mod tests {
    use concerto_ecs::{Entity, IntoSystem, System, With, World};

    use super::{MainCamera, spawn_main_camera};
    use crate::{director::CameraDirector, virtual_camera::VirtualCamera};

    fn run_spawn(world: &mut World) {
        let mut system = spawn_main_camera.into_system();
        system.initialize(world);
        system.run_and_apply((), world);
    }

    fn entities_with<T: concerto_ecs::Component>(world: &mut World) -> Vec<Entity> {
        world.query::<Entity, With<T>>().iter(world).collect()
    }

    fn director_world() -> World {
        let mut world = World::new();
        world.insert_resource(CameraDirector::default());
        world
    }

    #[test]
    fn a_director_without_a_main_camera_spawns_nothing() {
        let mut world = director_world();
        world.insert_resource(CameraDirector::default().with_main_camera(false));

        run_spawn(&mut world);

        assert!(entities_with::<MainCamera>(&mut world).is_empty());
        assert!(entities_with::<VirtualCamera>(&mut world).is_empty());
        assert_eq!(
            world
                .get_resource::<CameraDirector>()
                .unwrap()
                .main_camera(),
            None
        );
    }

    #[test]
    fn a_default_director_spawns_one_main_camera() {
        let mut world = director_world();

        run_spawn(&mut world);

        let main_cameras = entities_with::<MainCamera>(&mut world);
        assert_eq!(main_cameras.len(), 1);
        assert_eq!(
            world
                .get_resource::<CameraDirector>()
                .unwrap()
                .main_camera(),
            Some(main_cameras[0])
        );
    }
}
