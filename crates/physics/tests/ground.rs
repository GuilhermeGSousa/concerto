//! Ground probing (`PhysicsState::probe_ground`) and body velocity control.

use concerto_ecs::world::World;
use concerto_foundation::transform::Transform;
use glam::{Quat, Vec3};
mod common;
use common::{physics_world, register_bodies};

use concerto_physics::body::BodyId;
use concerto_physics::collider::Collider;
use concerto_physics::ground::GroundState;
use concerto_physics::physics_pipeline::PhysicsPipeline;
use concerto_physics::physics_state::PhysicsState;
use concerto_physics::rigid_body::{AllowedDofs, MotionType, RigidBody};

const MAX_SEPARATION: f32 = 0.05;
const MAX_SLOPE: f32 = std::f32::consts::PI * 50.0 / 180.0;

fn step(world: &mut World, pipeline: &mut PhysicsPipeline, steps: u32) {
    for _ in 0..steps {
        pipeline.step(world.get_resource_mut::<PhysicsState>().unwrap());
    }
}

#[test]
fn settled_sphere_reports_on_ground() {
    let mut world = physics_world();
    let mut pipeline = PhysicsPipeline::new();

    let floor = world.spawn((
        Collider::cuboid(100.0, 1.0, 100.0),
        Transform::from_translation_rotation(Vec3::ZERO, Quat::IDENTITY),
    ));
    let sphere = world.spawn((
        RigidBody::default(),
        Collider::sphere(1.0),
        Transform::from_translation_rotation(Vec3::new(0.0, 5.0, 0.0), Quat::IDENTITY),
    ));
    register_bodies(&mut world);

    step(&mut world, &mut pipeline, 120);

    let body = *world.get_component_for_entity::<BodyId>(sphere).unwrap();
    let state = world.get_resource::<PhysicsState>().unwrap();
    let ground = state.probe_ground(body, MAX_SEPARATION, MAX_SLOPE);

    let GroundState::OnGround(contact) = ground else {
        panic!("settled sphere should be OnGround, was {ground:?}");
    };
    assert!(
        contact.normal.y > 0.99,
        "flat floor normal should be +Y, was {}",
        contact.normal
    );
    assert_eq!(
        contact.entity,
        Some(floor),
        "contact should resolve to the floor entity"
    );
    assert!(
        contact.velocity.length() < 0.01,
        "static floor should have no velocity, was {}",
        contact.velocity
    );
}

#[test]
fn reports_in_air_before_landing() {
    let mut world = physics_world();

    world.spawn((
        Collider::cuboid(100.0, 1.0, 100.0),
        Transform::from_translation_rotation(Vec3::ZERO, Quat::IDENTITY),
    ));
    let sphere = world.spawn((
        RigidBody::default(),
        Collider::sphere(1.0),
        Transform::from_translation_rotation(Vec3::new(0.0, 10.0, 0.0), Quat::IDENTITY),
    ));
    register_bodies(&mut world);

    let body = *world.get_component_for_entity::<BodyId>(sphere).unwrap();
    let state = world.get_resource::<PhysicsState>().unwrap();
    let ground = state.probe_ground(body, MAX_SEPARATION, MAX_SLOPE);

    assert!(
        matches!(ground, GroundState::InAir),
        "sphere high above the floor should be InAir, was {ground:?}"
    );
}

#[test]
fn steep_slope_reports_on_steep_ground() {
    let mut world = physics_world();
    let mut pipeline = PhysicsPipeline::new();

    // A large box tilted 30° about Z; the capsule lands near its top center.
    world.spawn((
        Collider::cuboid(10.0, 1.0, 10.0),
        Transform::from_translation_rotation(
            Vec3::ZERO,
            Quat::from_rotation_z(30.0_f32.to_radians()),
        ),
    ));
    let capsule = world.spawn((
        RigidBody {
            allowed_dofs: AllowedDofs::TRANSLATION,
            ..Default::default()
        },
        Collider::capsule(0.5, 0.5),
        Transform::from_translation_rotation(Vec3::new(0.0, 5.0, 0.0), Quat::IDENTITY),
    ));
    register_bodies(&mut world);

    step(&mut world, &mut pipeline, 60);

    let body = *world.get_component_for_entity::<BodyId>(capsule).unwrap();
    let state = world.get_resource::<PhysicsState>().unwrap();
    let ground = state.probe_ground(body, MAX_SEPARATION, 20.0_f32.to_radians());

    assert!(
        matches!(ground, GroundState::OnSteepGround(_)),
        "a 30° slope with a 20° limit should be OnSteepGround, was {ground:?}"
    );
}

#[test]
fn set_velocity_wakes_a_sleeping_body() {
    let mut world = physics_world();
    let mut pipeline = PhysicsPipeline::new();

    world.spawn((
        Collider::cuboid(100.0, 1.0, 100.0),
        Transform::from_translation_rotation(Vec3::ZERO, Quat::IDENTITY),
    ));
    let sphere = world.spawn((
        RigidBody::default(),
        Collider::sphere(1.0),
        Transform::from_translation_rotation(Vec3::new(0.0, 3.0, 0.0), Quat::IDENTITY),
    ));
    register_bodies(&mut world);

    // 4 seconds: lands and sleeps (Jolt sleeps after ~0.5 s at rest).
    step(&mut world, &mut pipeline, 240);

    let body = *world.get_component_for_entity::<BodyId>(sphere).unwrap();
    let state = world.get_resource_mut::<PhysicsState>().unwrap();
    let start_x = state.body_transform(body).translation.x;
    state.set_linear_velocity(body, Vec3::new(5.0, 0.0, 0.0));

    step(&mut world, &mut pipeline, 30);

    let state = world.get_resource::<PhysicsState>().unwrap();
    let end_x = state.body_transform(body).translation.x;
    assert!(
        end_x > start_x + 1.0,
        "sleeping sphere should wake and move when given velocity, x went {start_x} -> {end_x}"
    );
}

#[test]
fn moving_platform_reports_ground_velocity() {
    let mut world = physics_world();
    let mut pipeline = PhysicsPipeline::new();

    let platform = world.spawn((
        RigidBody {
            motion_type: MotionType::Kinematic,
            ..Default::default()
        },
        Collider::cuboid(2.0, 0.5, 2.0),
        Transform::from_translation_rotation(Vec3::ZERO, Quat::IDENTITY),
    ));
    let sphere = world.spawn((
        RigidBody::default(),
        Collider::sphere(0.5),
        Transform::from_translation_rotation(Vec3::new(0.0, 2.0, 0.0), Quat::IDENTITY),
    ));
    register_bodies(&mut world);

    step(&mut world, &mut pipeline, 90);

    let platform_body = *world.get_component_for_entity::<BodyId>(platform).unwrap();
    let state = world.get_resource_mut::<PhysicsState>().unwrap();
    state.set_linear_velocity(platform_body, Vec3::new(2.0, 0.0, 0.0));

    step(&mut world, &mut pipeline, 1);

    let body = *world.get_component_for_entity::<BodyId>(sphere).unwrap();
    let state = world.get_resource::<PhysicsState>().unwrap();
    let ground = state.probe_ground(body, MAX_SEPARATION, MAX_SLOPE);

    let contact = ground
        .contact()
        .expect("sphere should be touching the platform");
    assert_eq!(contact.entity, Some(platform));
    assert!(
        (contact.velocity.x - 2.0).abs() < 0.2,
        "ground velocity should match the platform's, was {}",
        contact.velocity
    );
}
