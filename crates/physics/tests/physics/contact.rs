use crate::common::physics_world;
use concerto_ecs::entity::Entity;
use concerto_ecs::resource::{ResMut, Resource};
use concerto_ecs::signal::listener::{IntoListener, Listener};
use concerto_ecs::signal::On;
use concerto_ecs::system::executor::single_thread::SingleThreadedExecutor;
use concerto_ecs::system::schedule::Schedule;
use concerto_ecs::IntoSystemConfig;
use concerto_foundation::transform::Transform;
use glam::Vec3;

use concerto_physics::body::BodyId;
use concerto_physics::collider::{register_colliders, Collider};
use concerto_physics::contact::{report_contacts, ContactEnd, ContactStart, Contacts};
use concerto_physics::physics_pipeline::PhysicsPipeline;
use concerto_physics::physics_state::PhysicsState;
use concerto_physics::rigid_body::RigidBody;
use concerto_physics::step_simulation;

#[derive(Clone, Copy, PartialEq, Debug)]
struct Recorded {
    started: bool,
    other: Option<Entity>,
}

#[derive(Resource, Default)]
struct Changes(Vec<Recorded>);

fn record_start() -> Listener<ContactStart> {
    (|on: On<ContactStart>, mut changes: ResMut<Changes>| {
        changes.0.push(Recorded {
            started: true,
            other: on.signal().other,
        });
    })
    .into_listener()
}

fn record_end() -> Listener<ContactEnd> {
    (|on: On<ContactEnd>, mut changes: ResMut<Changes>| {
        changes.0.push(Recorded {
            started: false,
            other: on.signal().other,
        });
    })
    .into_listener()
}

#[test]
fn a_box_dropped_on_a_floor_reports_its_contact_until_it_is_lifted_away() {
    let mut world = physics_world();
    world.register_component::<Contacts>();
    world.register_component::<Listener<ContactStart>>();
    world.register_component::<Listener<ContactEnd>>();
    world.insert_resource(PhysicsPipeline::new());
    world.insert_resource(Changes::default());
    let floor = world.spawn((
        Collider::cuboid(10.0, 0.5, 10.0),
        Transform::from_translation(Vec3::new(0.0, -0.5, 0.0)),
    ));
    let quiet = world.spawn((
        RigidBody::default(),
        Collider::cuboid(0.5, 0.5, 0.5),
        Transform::from_translation(Vec3::new(5.0, 0.6, 0.0)),
    ));
    let dropped = world.spawn((
        RigidBody::default(),
        Collider::cuboid(0.5, 0.5, 0.5),
        Contacts::default(),
        Transform::from_translation(Vec3::new(0.0, 1.0, 0.0)),
        record_start(),
        record_end(),
    ));
    let mut schedule = Schedule::new();
    schedule.add_system((register_colliders, step_simulation, report_contacts).chain());
    let mut schedule = schedule.compile::<SingleThreadedExecutor>(&mut world);

    for _ in 0..40 {
        schedule.run(&mut world);
    }
    let started = Recorded {
        started: true,
        other: Some(floor),
    };
    assert_eq!(world.get_resource::<Changes>().unwrap().0, [started]);
    let contacts = world.get_component_for_entity::<Contacts>(dropped).unwrap();
    let contact = *contacts
        .with(floor)
        .expect("the resting box should list the floor");
    assert_eq!(contacts.iter().count(), 1);
    assert!(contact.normal.y > 0.99, "normal {}", contact.normal);
    assert!(contact.point.y.abs() < 0.05, "point {}", contact.point);
    let weight = 1000.0 * 9.81 * 0.033;
    assert!(
        contact.impulse > 0.5 * weight && contact.impulse < 2.0 * weight,
        "a resting 1000 kg box should press with about {weight} N·s a step, was {}",
        contact.impulse
    );
    assert!(
        world.get_component_for_entity::<Contacts>(quiet).is_none(),
        "a body that did not ask reports nothing"
    );

    let body = *world.get_component_for_entity::<BodyId>(dropped).unwrap();
    world
        .get_resource_mut::<PhysicsState>()
        .unwrap()
        .set_linear_velocity(body, Vec3::new(0.0, 8.0, 0.0));
    for _ in 0..5 {
        schedule.run(&mut world);
    }
    let ended = Recorded {
        started: false,
        other: Some(floor),
    };
    assert_eq!(world.get_resource::<Changes>().unwrap().0, [started, ended]);
    assert!(world
        .get_component_for_entity::<Contacts>(dropped)
        .unwrap()
        .is_empty());
}
