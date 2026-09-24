//! Regression test for the original API flaw: bodies used to be created
//! eagerly and never destroyed, so a "removed" collider kept colliding
//! forever. A body must leave the simulation with its `Collider` component.

mod common;
use common::{physics_world, register_bodies};

use concerto_foundation::transform::Transform;
use concerto_physics::body::BodyId;
use concerto_physics::collider::Collider;
use concerto_physics::physics_pipeline::PhysicsPipeline;
use concerto_physics::physics_state::PhysicsState;
use concerto_physics::rigid_body::RigidBody;
use glam::Vec3;

#[test]
fn despawned_collider_stops_colliding() {
    let mut world = physics_world();
    let mut pipeline = PhysicsPipeline::new();

    // Floor top at y = 1; unit sphere dropped from y = 5.
    let floor = world.spawn((
        Collider::cuboid(100.0, 1.0, 100.0),
        Transform::from_translation_rotation(Vec3::ZERO, Default::default()),
    ));
    let sphere = world.spawn((
        RigidBody::default(),
        Collider::sphere(1.0),
        Transform::from_translation_rotation(Vec3::new(0.0, 5.0, 0.0), Default::default()),
    ));
    register_bodies(&mut world);

    // One second: enough for the sphere to land, short enough that it cannot
    // have gone to sleep yet (Jolt sleeps after ~0.5 s at rest, and the drop
    // alone takes ~0.8 s).
    for _ in 0..60 {
        pipeline.step(world.get_resource_mut::<PhysicsState>().unwrap());
    }

    let body = *world
        .get_component_for_entity::<BodyId>(sphere)
        .expect("sphere should have a BodyId");
    let state = world.get_resource::<PhysicsState>().unwrap();
    let landed_y = state.body_transform(body).translation.y;
    assert!(
        (landed_y - 2.0).abs() < 0.5,
        "sphere should have landed on the floor near y = 2, was {landed_y}"
    );

    // A ray over empty floor (away from the sphere) hits it...
    let ray_origin = Vec3::new(30.0, 5.0, 0.0);
    let ray = Vec3::new(0.0, -10.0, 0.0);
    assert!(
        state.cast_ray(ray_origin, ray).is_some(),
        "floor should be hittable before despawn"
    );

    // ...but despawning the floor entity removes its body from the simulation.
    world.despawn(floor);

    let state = world.get_resource::<PhysicsState>().unwrap();
    assert!(
        state.cast_ray(ray_origin, ray).is_none(),
        "despawned floor should no longer be hittable"
    );

    // With the floor gone, the sphere falls straight through where it was.
    for _ in 0..60 {
        pipeline.step(world.get_resource_mut::<PhysicsState>().unwrap());
    }
    let end_y = world
        .get_resource::<PhysicsState>()
        .unwrap()
        .body_transform(body)
        .translation
        .y;
    assert!(
        end_y < landed_y - 3.0,
        "sphere should fall once the floor despawns, was y = {end_y}"
    );
}
