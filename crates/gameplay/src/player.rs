use concerto_director::VirtualCamera;
use concerto_ecs::{CommandQueue, Component, component::bundle::IntoBundle};
use concerto_foundation::transform::Transform;
use glam::{Quat, Vec3};

#[derive(Component)]
pub struct Player;

pub fn spawn_first_person_player<T: IntoBundle + 'static>(
    cmd: &mut CommandQueue,
    pos: Vec3,
    extra_components: T,
) {
    cmd.spawn((
        Player,
        Transform::from_translation_rotation(pos, Quat::IDENTITY),
        extra_components,
    ))
    .add_child((
        VirtualCamera::new(0),
        Transform::from_translation_rotation(Vec3::ZERO, Quat::IDENTITY),
    ));
}
