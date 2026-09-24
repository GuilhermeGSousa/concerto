//! `BodyId` must appear with the body `register_colliders` creates and go away
//! with the `Collider`, keeping the body-to-entity cache in sync so raycast
//! hits resolve in O(1).

mod common;
use common::{physics_world, register_bodies};

use concerto_core::transform::Transform;
use concerto_physics::body::BodyId;
use concerto_physics::collider::Collider;
use concerto_physics::physics_state::PhysicsState;
use glam::Vec3;

#[test]
fn body_id_tracks_component_lifecycle() {
    let mut world = physics_world();
    let entity = world.spawn((
        Collider::sphere(1.0),
        Transform::from_translation_rotation(Vec3::new(0.0, 5.0, 0.0), Default::default()),
    ));

    // Spawning is only the request; the body comes from the system.
    assert!(
        world.get_component_for_entity::<BodyId>(entity).is_none(),
        "a Collider should have no body before register_colliders runs"
    );

    register_bodies(&mut world);
    assert!(
        world.get_component_for_entity::<BodyId>(entity).is_some(),
        "register_colliders should insert a BodyId"
    );

    // Removal still goes through `Collider::on_remove`.
    world.remove_component::<Collider>(entity);
    assert!(
        world.get_component_for_entity::<BodyId>(entity).is_none(),
        "removing the Collider should remove its BodyId"
    );
}

#[test]
fn cache_cleared_on_despawn() {
    let mut world = physics_world();
    let floor = world.spawn((
        Collider::cuboid(100.0, 1.0, 100.0),
        Transform::from_translation_rotation(Vec3::ZERO, Default::default()),
    ));
    register_bodies(&mut world);

    let origin = Vec3::new(0.0, 5.0, 0.0);
    let ray = Vec3::new(0.0, -10.0, 0.0);

    let hit = world
        .get_resource::<PhysicsState>()
        .unwrap()
        .cast_ray(origin, ray)
        .expect("floor should be hittable while it exists");
    assert_eq!(
        hit.entity,
        Some(floor),
        "hit should resolve to the floor entity"
    );

    world.despawn(floor);
    assert!(
        world
            .get_resource::<PhysicsState>()
            .unwrap()
            .cast_ray(origin, ray)
            .is_none(),
        "despawning the floor should leave nothing to hit"
    );
}
