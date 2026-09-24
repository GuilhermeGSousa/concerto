//! End-to-end smoke test: a dynamic sphere dropped above a static floor should
//! fall under gravity and come to rest on top of the floor. Bodies are created
//! by spawning `Collider` components — never directly.

use concerto_core::transform::Transform;
use glam::{Quat, Vec3};
mod common;
use common::{physics_world, register_bodies};

use concerto_physics::body::BodyId;
use concerto_physics::collider::{Collider, ColliderOffset};
use concerto_physics::ground::GroundState;
use concerto_physics::physics_pipeline::PhysicsPipeline;
use concerto_physics::physics_state::PhysicsState;
use concerto_physics::rigid_body::{AllowedDofs, RigidBody};

#[test]
fn sphere_falls_and_rests_on_floor() {
    let mut world = physics_world();
    let mut pipeline = PhysicsPipeline::new();

    // Static floor: a 100 x 1 x 100 (half-extent) box centred at the origin, so
    // its top surface is at y = 1.
    world.spawn((
        Collider::cuboid(100.0, 1.0, 100.0),
        Transform::from_translation_rotation(Vec3::ZERO, Default::default()),
    ));

    // Dynamic sphere of radius 1, dropped from y = 10.
    let sphere = world.spawn((
        RigidBody::default(),
        Collider::sphere(1.0),
        Transform::from_translation_rotation(Vec3::new(0.0, 10.0, 0.0), Default::default()),
    ));
    register_bodies(&mut world);

    let body = *world
        .get_component_for_entity::<BodyId>(sphere)
        .expect("spawning a Collider should insert a BodyId");
    let state = world.get_resource::<PhysicsState>().unwrap();
    let start_y = state.body_transform(body).translation.y;
    assert!(
        (start_y - 10.0).abs() < 0.5,
        "sphere should start near y = 10, was {start_y}"
    );

    // Step ~3 seconds at 60 Hz.
    for _ in 0..180 {
        pipeline.step(world.get_resource_mut::<PhysicsState>().unwrap());
    }

    let end_y = world
        .get_resource::<PhysicsState>()
        .unwrap()
        .body_transform(body)
        .translation
        .y;

    // It must have fallen substantially...
    assert!(
        end_y < start_y - 5.0,
        "sphere should have fallen, y = {end_y}"
    );
    // ...and settled near the floor top (y = 1) + sphere radius (1) = 2.
    assert!(
        (end_y - 2.0).abs() < 0.5,
        "sphere should rest near y = 2 (floor top + radius), was {end_y}"
    );
}

/// The player-capsule setup: a capsule with the rotation DOFs locked must
/// land upright and stay upright.
#[test]
fn capsule_with_locked_rotation_rests_upright() {
    let mut world = physics_world();
    let mut pipeline = PhysicsPipeline::new();

    // Floor top at y = 1.
    world.spawn((
        Collider::cuboid(100.0, 1.0, 100.0),
        Transform::from_translation_rotation(Vec3::ZERO, Default::default()),
    ));

    // Capsule of total half-height 1 (cylinder half-height 0.5 + cap radius
    // 0.5), dropped from y = 5.
    let capsule = world.spawn((
        RigidBody {
            allowed_dofs: AllowedDofs::TRANSLATION,
            ..Default::default()
        },
        Collider::capsule(0.5, 0.5),
        Transform::from_translation_rotation(Vec3::new(0.0, 5.0, 0.0), Default::default()),
    ));
    register_bodies(&mut world);

    for _ in 0..180 {
        pipeline.step(world.get_resource_mut::<PhysicsState>().unwrap());
    }

    let body = *world
        .get_component_for_entity::<BodyId>(capsule)
        .expect("capsule should have a BodyId");
    let state = world.get_resource::<PhysicsState>().unwrap();
    let transform = state.body_transform(body);

    // Rest height: floor top (1) + capsule half-height (1) = 2.
    assert!(
        (transform.translation.y - 2.0).abs() < 0.5,
        "capsule should rest near y = 2 (floor top + half-height), was {}",
        transform.translation.y
    );
    assert!(
        transform.rotation.dot(Quat::IDENTITY).abs() > 0.999,
        "rotation-locked capsule should stay upright, was {:?}",
        transform.rotation
    );
}

/// With `ColliderOffset::bottom_origin`, the body origin is the shape's
/// bottom, so a settled character's transform sits at floor height — where a
/// GLTF skeleton root expects to be.
#[test]
fn bottom_origin_capsule_rests_with_origin_at_floor_height() {
    let mut world = physics_world();
    let mut pipeline = PhysicsPipeline::new();

    // Floor top at y = 1.
    world.spawn((
        Collider::cuboid(100.0, 1.0, 100.0),
        Transform::from_translation_rotation(Vec3::ZERO, Default::default()),
    ));

    let collider = Collider::capsule(0.5, 0.5);
    let offset = ColliderOffset::bottom_origin(&collider);
    let capsule = world.spawn((
        RigidBody {
            allowed_dofs: AllowedDofs::TRANSLATION,
            ..Default::default()
        },
        collider,
        offset,
        Transform::from_translation_rotation(Vec3::new(0.0, 5.0, 0.0), Default::default()),
    ));
    register_bodies(&mut world);

    for _ in 0..180 {
        pipeline.step(world.get_resource_mut::<PhysicsState>().unwrap());
    }

    let body = *world
        .get_component_for_entity::<BodyId>(capsule)
        .expect("capsule should have a BodyId");
    let state = world.get_resource::<PhysicsState>().unwrap();

    let origin_y = state.body_transform(body).translation.y;
    assert!(
        (origin_y - 1.0).abs() < 0.1,
        "bottom-origin capsule should rest with its origin on the floor top (y = 1), was {origin_y}"
    );

    // The ground probe must see through the offset wrapper.
    let ground = state.probe_ground(body, 0.05, 50.0_f32.to_radians());
    assert!(
        matches!(ground, GroundState::OnGround(_)),
        "offset capsule should probe OnGround, was {ground:?}"
    );
}
