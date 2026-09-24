pub mod assets;
pub mod blend;
pub use concerto_tasks as tasks;
pub mod geometry;
pub mod math;
pub mod time;
pub mod transform;
pub mod utils;

#[cfg(test)]
mod tests {
    use concerto_ecs::{component::Component, world::World};
    use glam::{Quat, Vec3};

    use crate::transform::Transform;

    #[derive(Component)]
    struct Player;

    #[derive(Component)]
    struct Health;

    #[test]
    fn test_add_transform() {
        let mut world = World::new();

        let e = world.spawn(Transform::from_translation_rotation(
            Vec3::ZERO,
            Quat::IDENTITY,
        ));

        world.insert(Player, e);
        world.insert(
            Transform::from_translation_rotation(Vec3::ZERO, Quat::IDENTITY),
            e,
        );
        world.insert(Health, e);
    }
}
