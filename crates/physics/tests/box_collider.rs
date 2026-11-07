use concerto_ecs::entity::Entity;
use concerto_ecs::system::executor::single_thread::SingleThreadedExecutor;
use concerto_ecs::system::schedule::{CompiledSchedule, Schedule};
use concerto_ecs::world::World;
use concerto_ecs::IntoSystemConfig;
use concerto_foundation::transform::Transform;
use glam::Vec3;
mod common;
use common::physics_world;

use concerto_physics::body::BodyId;
use concerto_physics::collider::{build_box_colliders, register_colliders, BoxCollider, Collider};
use concerto_physics::physics_pipeline::PhysicsPipeline;
use concerto_physics::physics_state::PhysicsState;
use concerto_physics::query::QueryFilter;
use concerto_physics::rigid_body::RigidBody;

struct BoxWorld {
    world: World,
    schedule: CompiledSchedule,
    pipeline: PhysicsPipeline,
}

impl BoxWorld {
    fn new() -> Self {
        let mut world = physics_world();
        world.register_component::<BoxCollider>();
        let mut schedule = Schedule::new();
        schedule.add_system(build_box_colliders);
        schedule.add_system(register_colliders.after(build_box_colliders));
        let schedule = schedule.compile::<SingleThreadedExecutor>(&mut world);
        Self {
            world,
            schedule,
            pipeline: PhysicsPipeline::new(),
        }
    }

    fn update(&mut self) {
        self.schedule.run(&mut self.world);
    }

    fn step(&mut self, steps: usize) {
        for _ in 0..steps {
            self.update();
            self.pipeline
                .step(self.world.get_resource_mut::<PhysicsState>().unwrap());
        }
    }

    fn height(&self, entity: Entity) -> f32 {
        let body = *self
            .world
            .get_component_for_entity::<BodyId>(entity)
            .expect("the entity should have a body");
        self.world
            .get_resource::<PhysicsState>()
            .unwrap()
            .body_transform(body)
            .translation
            .y
    }

    fn ray_down_hits(&self, origin: Vec3) -> bool {
        self.world
            .get_resource::<PhysicsState>()
            .unwrap()
            .cast_ray(origin, Vec3::new(0.0, -0.2, 0.0), QueryFilter::default())
            .is_some()
    }
}

#[test]
fn a_box_collider_builds_a_cuboid_and_rebuilds_it_when_edited() {
    let mut physics = BoxWorld::new();
    let block = physics.world.spawn((
        BoxCollider {
            half_extents: Vec3::new(5.0, 1.0, 5.0),
        },
        Transform::from_translation(Vec3::ZERO),
    ));
    let sphere = physics.world.spawn((
        RigidBody::default(),
        Collider::sphere(0.5),
        Transform::from_translation(Vec3::new(0.0, 4.0, 0.0)),
    ));
    physics.update();

    physics.step(120);
    let landed = physics.height(sphere);
    assert!(
        (landed - 1.5).abs() < 0.1,
        "the sphere should rest on the box top at y = 1, its centre near 1.5, was {landed}"
    );

    physics
        .world
        .get_component_for_entity_mut::<BoxCollider>(block)
        .unwrap()
        .half_extents = Vec3::new(5.0, 0.25, 5.0);
    physics.update();

    assert!(
        !physics.ray_down_hits(Vec3::new(3.0, 1.1, 0.0)),
        "a ray at the old top should miss once the box is lowered"
    );
    assert!(
        physics.ray_down_hits(Vec3::new(3.0, 0.35, 0.0)),
        "a ray just above the new top should hit it"
    );

    physics.step(120);
    let settled = physics.height(sphere);
    assert!(
        (settled - 0.75).abs() < 0.1,
        "the sphere should settle on the new top at y = 0.25, its centre near 0.75, was {settled}"
    );
}

#[test]
fn a_box_collider_with_a_zero_half_extent_builds_a_thin_plate() {
    let mut physics = BoxWorld::new();
    let plate = physics.world.spawn((
        BoxCollider {
            half_extents: Vec3::new(2.0, 0.0, 2.0),
        },
        Transform::from_translation(Vec3::ZERO),
    ));
    physics.update();

    let hit = physics
        .world
        .get_resource::<PhysicsState>()
        .unwrap()
        .cast_ray(
            Vec3::new(0.5, 0.2, 0.5),
            Vec3::new(0.0, -1.0, 0.0),
            QueryFilter::default(),
        )
        .expect("a ray down onto the flat box should hit it");
    assert_eq!(hit.entity, Some(plate));
    assert!(
        (hit.point.y - 0.001).abs() < 0.005,
        "the plate's top should sit at its clamped half extent, y = 0.001, was {}",
        hit.point.y
    );
}
