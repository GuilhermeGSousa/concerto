use std::sync::Arc;

use crate::common::{physics_world, register_bodies};
use concerto_foundation::transform::Transform;
use concerto_mesh::Mesh;
use glam::{Vec2, Vec3};

use concerto_physics::body::BodyId;
use concerto_physics::collider::Collider;
use concerto_physics::physics_state::PhysicsState;
use concerto_physics::query::QueryFilter;
use concerto_physics::shape::PhysicsShape;

const RADIUS: f32 = 0.5;

#[derive(Clone, Copy, PartialEq, Debug)]
enum Struck {
    Wall,
    Blocker,
    Plane,
}

struct Sweep {
    name: &'static str,
    origin: Vec3,
    direction: Vec3,
    ignore_blocker: bool,
    hit: Option<Hit>,
}

struct Hit {
    struck: Struck,
    fraction: f32,
    normal: Vec3,
}

#[test]
fn a_sphere_sweep_stops_a_radius_short_of_the_first_body_it_reaches_unless_ignored() {
    let mut world = physics_world();
    let wall = world.spawn((
        Collider::cuboid(0.5, 15.0, 5.0),
        Transform::from_translation(Vec3::new(5.0, 0.0, 0.0)),
    ));
    let blocker = world.spawn((
        Collider::cuboid(1.0, 1.0, 1.0),
        Transform::from_translation(Vec3::new(0.5, 10.0, 0.0)),
    ));
    let shape = PhysicsShape::from_mesh(&Mesh::plane(Vec2::splat(20.0))).unwrap();
    let floor = world.spawn((
        Collider::from_shape(Arc::new(shape)),
        Transform::from_translation(Vec3::new(0.0, -20.0, 0.0)),
    ));
    register_bodies(&mut world);
    let blocker_body = *world.get_component_for_entity::<BodyId>(blocker).unwrap();
    let state = world.get_resource::<PhysicsState>().unwrap();

    let sweeps = [
        Sweep {
            name: "into the wall",
            origin: Vec3::ZERO,
            direction: Vec3::new(10.0, 0.0, 0.0),
            ignore_blocker: false,
            hit: Some(Hit {
                struck: Struck::Wall,
                fraction: 0.4,
                normal: Vec3::NEG_X,
            }),
        },
        Sweep {
            name: "short of the wall",
            origin: Vec3::ZERO,
            direction: Vec3::new(3.0, 0.0, 0.0),
            ignore_blocker: false,
            hit: None,
        },
        Sweep {
            name: "from inside the blocker",
            origin: Vec3::new(0.0, 10.0, 0.0),
            direction: Vec3::new(10.0, 0.0, 0.0),
            ignore_blocker: false,
            hit: Some(Hit {
                struck: Struck::Blocker,
                fraction: 0.0,
                normal: Vec3::NEG_X,
            }),
        },
        Sweep {
            name: "from inside the ignored blocker",
            origin: Vec3::new(0.0, 10.0, 0.0),
            direction: Vec3::new(10.0, 0.0, 0.0),
            ignore_blocker: true,
            hit: Some(Hit {
                struck: Struck::Wall,
                fraction: 0.4,
                normal: Vec3::NEG_X,
            }),
        },
        Sweep {
            name: "down onto a mesh plane",
            origin: Vec3::new(0.0, -18.0, 0.0),
            direction: Vec3::new(0.0, -4.0, 0.0),
            ignore_blocker: false,
            hit: Some(Hit {
                struck: Struck::Plane,
                fraction: 0.375,
                normal: Vec3::Y,
            }),
        },
        Sweep {
            name: "up into a mesh plane",
            origin: Vec3::new(0.0, -22.0, 0.0),
            direction: Vec3::new(0.0, 4.0, 0.0),
            ignore_blocker: false,
            hit: Some(Hit {
                struck: Struck::Plane,
                fraction: 0.375,
                normal: Vec3::NEG_Y,
            }),
        },
    ];
    for sweep in sweeps {
        let ignore = [blocker_body];
        let filter = if sweep.ignore_blocker {
            QueryFilter::ignoring(&ignore)
        } else {
            QueryFilter::default()
        };
        let hit = state.cast_sphere(sweep.origin, RADIUS, sweep.direction, filter);
        match (hit, sweep.hit) {
            (None, None) => {}
            (Some(hit), Some(expected)) => {
                let entity = match expected.struck {
                    Struck::Wall => wall,
                    Struck::Blocker => blocker,
                    Struck::Plane => floor,
                };
                assert_eq!(hit.entity, Some(entity), "{}", sweep.name);
                assert!(
                    (hit.fraction - expected.fraction).abs() < 0.01,
                    "{}: fraction {}",
                    sweep.name,
                    hit.fraction
                );
                assert!(
                    hit.normal.dot(expected.normal) > 0.9,
                    "{}: normal {}",
                    sweep.name,
                    hit.normal
                );
            }
            (hit, _) => panic!("{}: got {hit:?}", sweep.name),
        }
    }
}
