use crate::common::{physics_world, register_bodies};
use concerto_ecs::entity::Entity;
use concerto_ecs::world::World;
use concerto_foundation::transform::Transform;
use glam::{Quat, Vec3};

use concerto_physics::body::BodyId;
use concerto_physics::collider::{Collider, ColliderOffset};
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

fn body(world: &World, entity: Entity) -> BodyId {
    *world.get_component_for_entity::<BodyId>(entity).unwrap()
}

fn floor(world: &mut World) -> Entity {
    world.spawn((Collider::cuboid(100.0, 1.0, 100.0), Transform::default()))
}

fn sphere(world: &mut World, height: f32) -> Entity {
    world.spawn((
        RigidBody::default(),
        Collider::sphere(1.0),
        Transform::from_translation(Vec3::new(0.0, height, 0.0)),
    ))
}

fn capsule(world: &mut World, offset: bool) -> Entity {
    let collider = Collider::capsule(0.5, 0.5);
    let body = RigidBody {
        allowed_dofs: AllowedDofs::TRANSLATION,
        ..Default::default()
    };
    let transform = Transform::from_translation(Vec3::new(0.0, 5.0, 0.0));
    if offset {
        let offset = ColliderOffset::bottom_origin(&collider);
        world.spawn((body, collider, offset, transform))
    } else {
        world.spawn((body, collider, transform))
    }
}

struct Stance {
    name: &'static str,
    max_slope: f32,
    run: fn(&mut World, &mut PhysicsPipeline) -> Probe,
    check: fn(&Probe, &GroundState) -> bool,
}

struct Probe {
    body: Entity,
    under: Option<Entity>,
}

#[test]
fn the_ground_probe_reports_what_a_body_stands_on() {
    let stances = [
        Stance {
            name: "a settled sphere on a floor",
            max_slope: MAX_SLOPE,
            run: |world, pipeline| {
                let floor = floor(world);
                let sphere = sphere(world, 5.0);
                register_bodies(world);
                step(world, pipeline, 120);
                Probe {
                    body: sphere,
                    under: Some(floor),
                }
            },
            check: |probe, ground| {
                matches!(ground, GroundState::OnGround(contact)
                    if contact.normal.y > 0.99
                        && contact.entity == probe.under
                        && contact.velocity.length() < 0.01)
            },
        },
        Stance {
            name: "a sphere above the floor",
            max_slope: MAX_SLOPE,
            run: |world, _| {
                floor(world);
                let sphere = sphere(world, 10.0);
                register_bodies(world);
                Probe {
                    body: sphere,
                    under: None,
                }
            },
            check: |_, ground| matches!(ground, GroundState::InAir),
        },
        Stance {
            name: "a capsule on a slope past its limit",
            max_slope: 20.0_f32.to_radians(),
            run: |world, pipeline| {
                world.spawn((
                    Collider::cuboid(10.0, 1.0, 10.0),
                    Transform::from_rotation(Quat::from_rotation_z(30.0_f32.to_radians())),
                ));
                let capsule = capsule(world, false);
                register_bodies(world);
                step(world, pipeline, 60);
                Probe {
                    body: capsule,
                    under: None,
                }
            },
            check: |_, ground| matches!(ground, GroundState::OnSteepGround(_)),
        },
        Stance {
            name: "a bottom-origin capsule on a floor",
            max_slope: MAX_SLOPE,
            run: |world, pipeline| {
                floor(world);
                let capsule = capsule(world, true);
                register_bodies(world);
                step(world, pipeline, 180);
                Probe {
                    body: capsule,
                    under: None,
                }
            },
            check: |_, ground| matches!(ground, GroundState::OnGround(_)),
        },
        Stance {
            name: "a sphere on a moving platform",
            max_slope: MAX_SLOPE,
            run: |world, pipeline| {
                let platform = world.spawn((
                    RigidBody {
                        motion_type: MotionType::Kinematic,
                        ..Default::default()
                    },
                    Collider::cuboid(2.0, 0.5, 2.0),
                    Transform::default(),
                ));
                let sphere = world.spawn((
                    RigidBody::default(),
                    Collider::sphere(0.5),
                    Transform::from_translation(Vec3::new(0.0, 2.0, 0.0)),
                ));
                register_bodies(world);
                step(world, pipeline, 90);
                let platform_body = body(world, platform);
                world
                    .get_resource_mut::<PhysicsState>()
                    .unwrap()
                    .set_linear_velocity(platform_body, Vec3::new(2.0, 0.0, 0.0));
                step(world, pipeline, 1);
                Probe {
                    body: sphere,
                    under: Some(platform),
                }
            },
            check: |probe, ground| {
                ground.contact().is_some_and(|contact| {
                    contact.entity == probe.under && (contact.velocity.x - 2.0).abs() < 0.2
                })
            },
        },
    ];
    for stance in stances {
        let mut world = physics_world();
        let mut pipeline = PhysicsPipeline::new();
        let probe = (stance.run)(&mut world, &mut pipeline);

        let ground = world.get_resource::<PhysicsState>().unwrap().probe_ground(
            body(&world, probe.body),
            MAX_SEPARATION,
            stance.max_slope,
        );

        assert!(
            (stance.check)(&probe, &ground),
            "{}: {ground:?}",
            stance.name
        );
    }
}

#[test]
fn set_velocity_wakes_a_sleeping_body() {
    let mut world = physics_world();
    let mut pipeline = PhysicsPipeline::new();
    floor(&mut world);
    let sphere = sphere(&mut world, 3.0);
    register_bodies(&mut world);
    step(&mut world, &mut pipeline, 240);

    let body = body(&world, sphere);
    let state = world.get_resource_mut::<PhysicsState>().unwrap();
    let start_x = state.body_transform(body).translation.x;
    state.set_linear_velocity(body, Vec3::new(5.0, 0.0, 0.0));

    step(&mut world, &mut pipeline, 30);

    let end_x = world
        .get_resource::<PhysicsState>()
        .unwrap()
        .body_transform(body)
        .translation
        .x;
    assert!(end_x > start_x + 1.0, "x went {start_x} -> {end_x}");
}
