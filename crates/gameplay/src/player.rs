use concerto_director::VirtualCamera;
use concerto_ecs::{CommandQueue, Component, component::bundle::IntoBundle};
use concerto_foundation::transform::Transform;
use glam::{Quat, Vec3};

/// Marks the free-flying player that
/// [`first_person_player_fly`](crate::movement::first_person_player_fly) moves.
#[derive(Component)]
pub struct Player;

/// Spawns a [`Player`] at `pos` with `extra_components`, and a child [`VirtualCamera`] at its
/// origin.
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
