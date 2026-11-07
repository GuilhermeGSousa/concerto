use std::sync::Arc;

use concerto_ecs::entity::Entity;
use concerto_ecs::system::executor::single_thread::SingleThreadedExecutor;
use concerto_ecs::system::schedule::{CompiledSchedule, Schedule};
use concerto_ecs::world::World;
use concerto_foundation::transform::Transform;
use concerto_mesh::primitives::plane;
use glam::{Vec2, Vec3};
mod common;
use common::physics_world;

use concerto_physics::body::BodyId;
use concerto_physics::collider::{register_colliders, Collider};
use concerto_physics::ground::GroundState;
use concerto_physics::physics_pipeline::PhysicsPipeline;
use concerto_physics::physics_state::PhysicsState;
use concerto_physics::query::QueryFilter;
use concerto_physics::rigid_body::RigidBody;
use concerto_physics::sensor::Sensor;
use concerto_physics::shape::{MeshCollider, PhysicsShape};

const MAX_SEPARATION: f32 = 0.05;
const MAX_SLOPE: f32 = std::f32::consts::PI * 50.0 / 180.0;

struct SensorWorld {
    world: World,
    schedule: CompiledSchedule,
    pipeline: PhysicsPipeline,
}

impl SensorWorld {
    fn new() -> Self {
        let mut world = physics_world();
        world.register_component::<Sensor>();
        world.register_component::<MeshCollider>();
        let mut schedule = Schedule::new();
        schedule.add_system(register_colliders);
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

    fn state(&self) -> &PhysicsState {
        self.world.get_resource::<PhysicsState>().unwrap()
    }

    fn state_mut(&mut self) -> &mut PhysicsState {
        self.world.get_resource_mut::<PhysicsState>().unwrap()
    }

    fn body(&self, entity: Entity) -> BodyId {
        *self
            .world
            .get_component_for_entity::<BodyId>(entity)
            .expect("the entity should have a body")
    }

    fn position(&self, entity: Entity) -> Vec3 {
        self.state().body_transform(self.body(entity)).translation
    }
}

fn at(translation: Vec3) -> Transform {
    Transform::from_translation(translation)
}

#[test]
fn queries_pass_through_sensors() {
    let mut physics = SensorWorld::new();
    let wall = physics.world.spawn((
        Collider::cuboid(0.5, 5.0, 5.0),
        at(Vec3::new(5.0, 0.0, 0.0)),
    ));
    physics
        .world
        .spawn((Collider::cuboid(2.0, 2.0, 2.0), Sensor, at(Vec3::ZERO)));
    physics.update();

    let origin = Vec3::new(-5.0, 0.0, 0.0);
    let direction = Vec3::new(20.0, 0.0, 0.0);
    let ray = physics
        .state()
        .cast_ray(origin, direction, QueryFilter::default())
        .expect("the ray should hit the wall behind the sensor");
    assert_eq!(ray.entity, Some(wall));
    assert!(
        (ray.point.x - 4.5).abs() < 0.01,
        "the ray should stop at the wall's face, x = 4.5, was {}",
        ray.point.x
    );

    let sweep = physics
        .state()
        .cast_sphere(origin, 0.25, direction, QueryFilter::default())
        .expect("the sweep should hit the wall behind the sensor");
    assert_eq!(sweep.entity, Some(wall));
}

#[test]
fn the_ground_probe_ignores_sensors() {
    let mut physics = SensorWorld::new();
    let floor = physics.world.spawn((
        Collider::cuboid(10.0, 0.5, 10.0),
        at(Vec3::new(0.0, -0.5, 0.0)),
    ));
    physics
        .world
        .spawn((Collider::cuboid(3.0, 3.0, 3.0), Sensor, at(Vec3::ZERO)));
    let resting = physics.world.spawn((
        RigidBody::default(),
        Collider::sphere(0.5),
        at(Vec3::new(-1.5, 0.5, 0.0)),
    ));
    let airborne = physics.world.spawn((
        RigidBody::default(),
        Collider::sphere(0.5),
        at(Vec3::new(1.5, 2.0, 0.0)),
    ));
    physics.update();

    let in_air = physics
        .state()
        .probe_ground(physics.body(airborne), MAX_SEPARATION, MAX_SLOPE);
    assert!(
        matches!(in_air, GroundState::InAir),
        "a sphere in the air inside a sensor should be InAir, was {in_air:?}"
    );

    physics.step(30);
    let ground = physics
        .state()
        .probe_ground(physics.body(resting), MAX_SEPARATION, MAX_SLOPE);
    let GroundState::OnGround(contact) = ground else {
        panic!("a sphere resting inside a sensor should be OnGround, was {ground:?}");
    };
    assert_eq!(contact.entity, Some(floor));
}

#[test]
fn a_sensor_on_a_dynamic_body_does_not_fall() {
    let mut physics = SensorWorld::new();
    let sensor = physics.world.spawn((
        RigidBody::default(),
        Collider::sphere(1.0),
        Sensor,
        at(Vec3::new(0.0, 5.0, 0.0)),
    ));
    physics.update();

    physics.step(60);
    let height = physics.position(sensor).y;
    assert!(
        (height - 5.0).abs() < 1e-3,
        "a sensor is never dynamic, so it should stay at y = 5, was {height}"
    );
}

#[test]
fn a_static_sensor_ignores_a_velocity() {
    let mut physics = SensorWorld::new();
    let start = Vec3::new(1.0, 2.0, 3.0);
    let sensor = physics
        .world
        .spawn((Collider::cuboid(1.0, 1.0, 1.0), Sensor, at(start)));
    physics.update();

    let body = physics.body(sensor);
    physics
        .state_mut()
        .set_linear_velocity(body, Vec3::new(4.0, 0.0, 0.0));
    physics.step(60);

    let position = physics.position(sensor);
    assert!(
        position.distance(start) < 1e-3,
        "a static sensor should stay where it spawned, at {start}, was {position}"
    );
}

#[test]
fn a_sensor_on_a_mesh_collider_is_ignored() {
    let mut physics = SensorWorld::new();
    let shape = PhysicsShape::from_mesh(&plane(Vec2::splat(20.0))).expect("plane mesh shape");
    physics.world.spawn((
        Collider::from_shape(Arc::new(shape)),
        MeshCollider,
        Sensor,
        at(Vec3::ZERO),
    ));
    let sphere = physics.world.spawn((
        RigidBody::default(),
        Collider::sphere(0.5),
        at(Vec3::new(0.0, 3.0, 0.0)),
    ));
    physics.update();

    physics.step(120);
    let height = physics.position(sphere).y;
    assert!(
        (height - 0.5).abs() < 0.1,
        "the sphere should land on the mesh, its centre near 0.5, was {height}"
    );
}
