use crate::common::{physics_world, register_bodies};
use concerto_foundation::transform::Transform;
use glam::Vec3;

use concerto_physics::body::BodyId;
use concerto_physics::collider::Collider;
use concerto_physics::physics_pipeline::PhysicsPipeline;
use concerto_physics::physics_state::PhysicsState;
use concerto_physics::rigid_body::RigidBody;
use concerto_physics::settings::PhysicsSettings;

fn bullet_x_after_a_second(settings: PhysicsSettings) -> f32 {
    let mut world = physics_world();
    let mut pipeline = PhysicsPipeline::new();
    world.spawn((Collider::cuboid(0.1, 50.0, 50.0), Transform::default()));
    let bullet = world.spawn((
        RigidBody::default(),
        Collider::sphere(0.1),
        Transform::from_translation(Vec3::new(-0.8, 0.0, 0.0)),
    ));
    register_bodies(&mut world);
    let body = *world.get_component_for_entity::<BodyId>(bullet).unwrap();
    let state = world.get_resource_mut::<PhysicsState>().unwrap();
    state.set_linear_velocity(body, Vec3::new(1.5 / 0.033, 0.0, 0.0));
    for _ in 0..30 {
        pipeline.step_with(state, &settings);
    }
    state.body_transform(body).translation.x
}

#[test]
fn a_fast_body_tunnels_a_thin_wall_in_one_collision_step_and_stops_in_two() {
    let coarse = bullet_x_after_a_second(PhysicsSettings {
        max_collision_step: 0.033,
    });
    assert!(
        coarse > 0.2,
        "one 33 ms collision step should carry the body through the wall, was at x = {coarse}"
    );

    let fine = bullet_x_after_a_second(PhysicsSettings::default());
    assert!(
        fine < -0.1,
        "collision steps of at most 1/60 s should stop the body at the wall, was at x = {fine}"
    );
}
