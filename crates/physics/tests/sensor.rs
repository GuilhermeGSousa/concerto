use std::sync::Arc;

use concerto_ecs::entity::Entity;
use concerto_ecs::resource::{ResMut, Resource};
use concerto_ecs::signal::listener::{IntoListener, Listener};
use concerto_ecs::signal::On;
use concerto_ecs::system::executor::single_thread::SingleThreadedExecutor;
use concerto_ecs::system::schedule::{CompiledSchedule, Schedule};
use concerto_ecs::world::World;
use concerto_ecs::IntoSystemConfig;
use concerto_foundation::transform::Transform;
use concerto_mesh::primitives::plane;
use concerto_mesh::MeshComponent;
use glam::{Vec2, Vec3};
mod common;
use common::physics_world;

use concerto_physics::backend::SensorContactKind;
use concerto_physics::body::BodyId;
use concerto_physics::collider::{build_box_colliders, register_colliders, BoxCollider, Collider};
use concerto_physics::ground::GroundState;
use concerto_physics::physics_pipeline::PhysicsPipeline;
use concerto_physics::physics_state::PhysicsState;
use concerto_physics::query::QueryFilter;
use concerto_physics::rigid_body::{MotionType, RigidBody};
use concerto_physics::sensor::{report_sensor_contacts, Sensor, SensorEnter, SensorExit};
use concerto_physics::shape::{MeshCollider, PhysicsShape};
use concerto_physics::step_simulation;

const MAX_SEPARATION: f32 = 0.05;
const MAX_SLOPE: f32 = std::f32::consts::PI * 50.0 / 180.0;

#[derive(Clone, Copy, PartialEq, Debug)]
struct Recorded {
    kind: SensorContactKind,
    other: Entity,
}

#[derive(Resource, Default)]
struct Contacts(Vec<Recorded>);

fn record_enter() -> Listener<SensorEnter> {
    (|on: On<SensorEnter>, mut contacts: ResMut<Contacts>| {
        contacts.0.push(Recorded {
            kind: SensorContactKind::Enter,
            other: on.signal().other,
        });
    })
    .into_listener()
}

fn record_exit() -> Listener<SensorExit> {
    (|on: On<SensorExit>, mut contacts: ResMut<Contacts>| {
        contacts.0.push(Recorded {
            kind: SensorContactKind::Exit,
            other: on.signal().other,
        });
    })
    .into_listener()
}

fn enter(other: Entity) -> Recorded {
    Recorded {
        kind: SensorContactKind::Enter,
        other,
    }
}

fn exit(other: Entity) -> Recorded {
    Recorded {
        kind: SensorContactKind::Exit,
        other,
    }
}

struct SensorWorld {
    world: World,
    registration: CompiledSchedule,
    fixed_step: CompiledSchedule,
}

impl SensorWorld {
    fn new() -> Self {
        let mut world = physics_world();
        world.register_component::<Sensor>();
        world.register_component::<MeshCollider>();
        world.register_component::<MeshComponent>();
        world.register_component::<BoxCollider>();
        world.register_component::<Listener<SensorEnter>>();
        world.register_component::<Listener<SensorExit>>();
        world.insert_resource(PhysicsPipeline::new());
        world.insert_resource(Contacts::default());
        let mut registration = Schedule::new();
        registration.add_system((build_box_colliders, register_colliders).chain());
        let registration = registration.compile::<SingleThreadedExecutor>(&mut world);
        let mut fixed_step = Schedule::new();
        fixed_step.add_system(
            (
                build_box_colliders,
                register_colliders,
                step_simulation,
                report_sensor_contacts,
            )
                .chain(),
        );
        let fixed_step = fixed_step.compile::<SingleThreadedExecutor>(&mut world);
        Self {
            world,
            registration,
            fixed_step,
        }
    }

    fn update(&mut self) {
        self.registration.run(&mut self.world);
    }

    fn step(&mut self, steps: usize) {
        for _ in 0..steps {
            self.fixed_step.run(&mut self.world);
        }
    }

    fn contacts(&self) -> &[Recorded] {
        &self.world.get_resource::<Contacts>().unwrap().0
    }

    fn clear_contacts(&mut self) {
        self.world.get_resource_mut::<Contacts>().unwrap().0.clear();
    }

    fn spawn_sensor(&mut self, collider: Collider, transform: Transform) -> Entity {
        self.world
            .spawn((collider, Sensor, transform, record_enter(), record_exit()))
    }

    fn spawn_kinematic_sensor(&mut self, collider: Collider, transform: Transform) -> Entity {
        self.world.spawn((
            RigidBody {
                motion_type: MotionType::Kinematic,
                ..RigidBody::default()
            },
            collider,
            Sensor,
            transform,
            record_enter(),
            record_exit(),
        ))
    }

    fn set_velocity(&mut self, entity: Entity, velocity: Vec3) {
        let body = self.body(entity);
        self.state_mut().set_linear_velocity(body, velocity);
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

#[test]
fn a_mesh_collider_with_a_box_collider_is_a_sensor() {
    let mut physics = SensorWorld::new();
    physics.world.spawn((
        MeshComponent::default(),
        MeshCollider,
        BoxCollider {
            half_extents: Vec3::new(2.0, 1.0, 2.0),
        },
        Sensor,
        at(Vec3::new(0.0, 5.0, 0.0)),
        record_enter(),
        record_exit(),
    ));
    let sphere = physics.world.spawn((
        RigidBody::default(),
        Collider::sphere(0.5),
        at(Vec3::new(0.0, 8.0, 0.0)),
    ));
    physics.update();

    physics.step(90);
    assert_eq!(physics.contacts(), [enter(sphere), exit(sphere)]);
    let height = physics.position(sphere).y;
    assert!(
        height < 3.5,
        "the box wins over the mesh, so the sphere should fall through the sensor, below y = 3.5, was {height}"
    );
}

#[test]
fn a_falling_sphere_enters_and_leaves_a_sensor() {
    let mut physics = SensorWorld::new();
    physics.spawn_sensor(
        Collider::cuboid(2.0, 1.0, 2.0),
        at(Vec3::new(0.0, 5.0, 0.0)),
    );
    let sphere = physics.world.spawn((
        RigidBody::default(),
        Collider::sphere(0.5),
        at(Vec3::new(0.0, 8.0, 0.0)),
    ));
    physics.update();

    physics.step(90);
    assert_eq!(physics.contacts(), [enter(sphere), exit(sphere)]);
    let height = physics.position(sphere).y;
    assert!(
        height < 3.5,
        "the sphere should fall on through the sensor, below y = 3.5, was {height}"
    );
}

#[test]
fn a_sphere_asleep_inside_a_sensor_does_not_leave_it() {
    let mut physics = SensorWorld::new();
    physics.world.spawn((
        Collider::cuboid(10.0, 0.5, 10.0),
        at(Vec3::new(0.0, -0.5, 0.0)),
    ));
    physics.spawn_sensor(Collider::cuboid(3.0, 3.0, 3.0), at(Vec3::ZERO));
    let sphere = physics.world.spawn((
        RigidBody::default(),
        Collider::sphere(0.5),
        at(Vec3::new(0.0, 0.5, 0.0)),
    ));
    physics.update();

    physics.step(360);
    assert_eq!(physics.contacts(), [enter(sphere)]);
}

#[test]
fn a_kinematic_body_moved_through_a_sensor_enters_and_leaves_it() {
    let mut physics = SensorWorld::new();
    physics.spawn_sensor(Collider::cuboid(1.0, 1.0, 1.0), at(Vec3::ZERO));
    let mover = physics.world.spawn((
        RigidBody {
            motion_type: MotionType::Kinematic,
            ..RigidBody::default()
        },
        Collider::cuboid(0.5, 0.5, 0.5),
        at(Vec3::new(-4.0, 0.0, 0.0)),
    ));
    physics.update();

    physics.set_velocity(mover, Vec3::new(4.0, 0.0, 0.0));
    physics.step(120);
    assert_eq!(physics.contacts(), [enter(mover), exit(mover)]);
}

#[test]
fn a_kinematic_sensor_reports_a_resting_sphere_and_not_the_floor() {
    let mut physics = SensorWorld::new();
    physics.world.spawn((
        Collider::cuboid(20.0, 0.5, 20.0),
        at(Vec3::new(0.0, -0.5, 0.0)),
    ));
    let sphere = physics.world.spawn((
        RigidBody::default(),
        Collider::sphere(0.5),
        at(Vec3::new(0.0, 0.5, 0.0)),
    ));
    let sensor = physics.spawn_kinematic_sensor(
        Collider::cuboid(1.0, 1.0, 1.0),
        at(Vec3::new(-5.0, 0.5, 0.0)),
    );
    physics.update();

    physics.step(150);
    assert!(
        physics.contacts().is_empty(),
        "the sensor should hear nothing before it moves, got {:?}",
        physics.contacts()
    );
    physics.set_velocity(sensor, Vec3::new(4.0, 0.0, 0.0));
    physics.step(150);
    assert_eq!(physics.contacts(), [enter(sphere), exit(sphere)]);
}

#[test]
fn sensors_do_not_detect_each_other() {
    let mut physics = SensorWorld::new();
    physics.world.spawn((
        Collider::cuboid(20.0, 0.5, 20.0),
        at(Vec3::new(0.0, -0.5, 0.0)),
    ));
    physics.spawn_sensor(
        Collider::cuboid(1.0, 1.0, 1.0),
        at(Vec3::new(0.0, 0.5, 0.0)),
    );
    let sphere = physics.world.spawn((
        RigidBody::default(),
        Collider::sphere(0.5),
        at(Vec3::new(0.0, 0.5, 0.0)),
    ));
    let moving = physics.spawn_kinematic_sensor(
        Collider::cuboid(1.0, 1.0, 1.0),
        at(Vec3::new(-3.0, 0.5, 0.0)),
    );
    physics.update();

    physics.set_velocity(moving, Vec3::new(3.0, 0.0, 0.0));
    physics.step(120);
    assert_eq!(
        physics.contacts(),
        [enter(sphere), enter(sphere), exit(sphere)],
        "the sensors should record only the sphere, never each other"
    );
}

#[test]
fn replacing_a_sensors_collider_exits_and_enters_again() {
    let mut physics = SensorWorld::new();
    physics.world.spawn((
        Collider::cuboid(20.0, 0.5, 20.0),
        at(Vec3::new(0.0, -0.5, 0.0)),
    ));
    let sensor = physics.spawn_sensor(Collider::cuboid(2.0, 2.0, 2.0), at(Vec3::ZERO));
    let sphere = physics.world.spawn((
        RigidBody::default(),
        Collider::sphere(0.5),
        at(Vec3::new(0.0, 0.5, 0.0)),
    ));
    physics.update();
    physics.step(10);
    assert_eq!(physics.contacts(), [enter(sphere)]);

    physics
        .world
        .insert(Collider::cuboid(2.0, 2.0, 2.0), sensor);
    physics.step(10);
    assert_eq!(
        physics.contacts(),
        [enter(sphere), exit(sphere), enter(sphere)]
    );
}

#[test]
fn replacing_the_collider_of_a_body_inside_a_sensor_exits_and_enters_again() {
    let mut physics = SensorWorld::new();
    physics.world.spawn((
        Collider::cuboid(20.0, 0.5, 20.0),
        at(Vec3::new(0.0, -0.5, 0.0)),
    ));
    physics.spawn_sensor(Collider::cuboid(2.0, 2.0, 2.0), at(Vec3::ZERO));
    let sphere = physics.world.spawn((
        RigidBody::default(),
        Collider::sphere(0.5),
        at(Vec3::new(0.0, 0.5, 0.0)),
    ));
    physics.update();
    physics.step(10);
    assert_eq!(physics.contacts(), [enter(sphere)]);

    physics.world.insert(Collider::sphere(0.5), sphere);
    physics.step(10);
    assert_eq!(
        physics.contacts(),
        [enter(sphere), exit(sphere), enter(sphere)]
    );
}

#[test]
fn despawning_a_body_inside_a_sensor_exits_it() {
    let mut physics = SensorWorld::new();
    physics.world.spawn((
        Collider::cuboid(20.0, 0.5, 20.0),
        at(Vec3::new(0.0, -0.5, 0.0)),
    ));
    physics.spawn_sensor(Collider::cuboid(2.0, 2.0, 2.0), at(Vec3::ZERO));
    let sphere = physics.world.spawn((
        RigidBody::default(),
        Collider::sphere(0.5),
        at(Vec3::new(0.0, 0.5, 0.0)),
    ));
    physics.update();
    physics.step(10);
    assert_eq!(physics.contacts(), [enter(sphere)]);

    physics.world.despawn(sphere);
    physics.step(30);
    assert_eq!(physics.contacts(), [enter(sphere), exit(sphere)]);

    let sensor = physics.spawn_sensor(
        Collider::cuboid(2.0, 2.0, 2.0),
        at(Vec3::new(8.0, 0.0, 0.0)),
    );
    let inside = physics.world.spawn((
        RigidBody::default(),
        Collider::sphere(0.5),
        at(Vec3::new(8.0, 0.5, 0.0)),
    ));
    physics.update();
    physics.step(10);
    assert_eq!(physics.contacts()[2..], [enter(inside)]);
    physics.clear_contacts();

    physics.world.despawn(sensor);
    physics.step(30);
    assert!(
        physics.contacts().is_empty(),
        "a despawned sensor should hear nothing, got {:?}",
        physics.contacts()
    );
    let height = physics.position(inside).y;
    assert!(
        (height - 0.5).abs() < 0.1,
        "the sphere should still rest on the floor, its centre near 0.5, was {height}"
    );
}
