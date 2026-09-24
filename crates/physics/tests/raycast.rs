//! Raycast smoke tests. `cast_ray` reports the closest hit with its fraction,
//! hit point, surface normal, and the entity owning the hit `Collider`; rays
//! that miss report nothing.

use concerto_core::transform::Transform;
use concerto_ecs::entity::Entity;
use concerto_ecs::world::World;
use glam::Vec3;
mod common;
use common::{physics_world, register_bodies};

use concerto_physics::collider::Collider;
use concerto_physics::physics_state::PhysicsState;
use concerto_physics::rigid_body::RigidBody;

/// A world with one dynamic unit sphere centred at (0, 5, 0).
fn world_with_sphere() -> (World, Entity) {
    let mut world = physics_world();

    let transform =
        Transform::from_translation_rotation(Vec3::new(0.0, 5.0, 0.0), Default::default());
    let entity = world.spawn((RigidBody::default(), Collider::sphere(1.0), transform));
    register_bodies(&mut world);

    (world, entity)
}

#[test]
fn ray_hits_sphere_from_above() {
    let (world, entity) = world_with_sphere();
    let state = world.get_resource::<PhysicsState>().unwrap();

    // Cast straight down from y = 10 with a reach of 8 units: the sphere top
    // at y = 6 is 4 units away, so the hit fraction should be 0.5.
    let hit = state
        .cast_ray(Vec3::new(0.0, 10.0, 0.0), Vec3::new(0.0, -8.0, 0.0))
        .expect("ray pointing at the sphere should hit it");

    assert_eq!(
        hit.entity,
        Some(entity),
        "hit should resolve to the sphere entity"
    );
    assert!(
        (hit.fraction - 0.5).abs() < 0.01,
        "expected hit fraction ~0.5, got {}",
        hit.fraction
    );
    assert!(
        (hit.point - Vec3::new(0.0, 6.0, 0.0)).length() < 0.05,
        "expected hit point near (0, 6, 0), got {}",
        hit.point
    );
    assert!(
        (hit.normal - Vec3::Y).length() < 0.01,
        "expected an upward surface normal at the sphere top, got {}",
        hit.normal
    );
}

#[test]
fn ray_misses_when_pointing_away() {
    let (world, _) = world_with_sphere();
    let state = world.get_resource::<PhysicsState>().unwrap();

    // Pointing up, away from the sphere.
    let hit = state.cast_ray(Vec3::new(0.0, 10.0, 0.0), Vec3::new(0.0, 8.0, 0.0));
    assert!(hit.is_none(), "upward ray should not hit the sphere");

    // Pointing down but too short to reach it (sphere top is 4 units away).
    let hit = state.cast_ray(Vec3::new(0.0, 10.0, 0.0), Vec3::new(0.0, -2.0, 0.0));
    assert!(hit.is_none(), "short ray should stop before the sphere");
}

#[test]
fn ray_hits_static_geometry_entity() {
    // Static geometry is spawned as an entity like everything else, so hits
    // against it resolve to that entity too.
    let mut world = physics_world();

    let floor = world.spawn((
        Collider::cuboid(100.0, 1.0, 100.0),
        Transform::from_translation_rotation(Vec3::ZERO, Default::default()),
    ));
    register_bodies(&mut world);

    // Floor top is at y = 1, i.e. 4 of the ray's 10 units away.
    let state = world.get_resource::<PhysicsState>().unwrap();
    let hit = state
        .cast_ray(Vec3::new(0.0, 5.0, 0.0), Vec3::new(0.0, -10.0, 0.0))
        .expect("ray pointing at the floor should hit it");
    assert_eq!(
        hit.entity,
        Some(floor),
        "static colliders resolve to their entity"
    );
    assert!(
        (hit.fraction - 0.4).abs() < 0.01,
        "expected hit fraction ~0.4, got {}",
        hit.fraction
    );
}
