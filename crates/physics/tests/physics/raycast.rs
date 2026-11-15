use crate::common::{physics_world, register_bodies};
use concerto_foundation::transform::Transform;
use glam::Vec3;

use concerto_physics::body::BodyId;
use concerto_physics::collider::Collider;
use concerto_physics::physics_state::PhysicsState;
use concerto_physics::query::QueryFilter;
use concerto_physics::rigid_body::RigidBody;

struct Ray {
    name: &'static str,
    origin: Vec3,
    direction: Vec3,
    ignore_sphere: bool,
    hit: Option<Hit>,
}

struct Hit {
    sphere: bool,
    fraction: f32,
    point: Vec3,
}

#[test]
fn a_ray_reports_the_first_body_it_reaches_unless_that_body_is_ignored() {
    let mut world = physics_world();
    let sphere = world.spawn((
        RigidBody::default(),
        Collider::sphere(1.0),
        Transform::from_translation(Vec3::new(0.0, 5.0, 0.0)),
    ));
    let floor = world.spawn((Collider::cuboid(100.0, 1.0, 100.0), Transform::default()));
    register_bodies(&mut world);
    let sphere_body = *world.get_component_for_entity::<BodyId>(sphere).unwrap();
    let state = world.get_resource::<PhysicsState>().unwrap();

    let rays = [
        Ray {
            name: "down onto the sphere",
            origin: Vec3::new(0.0, 10.0, 0.0),
            direction: Vec3::new(0.0, -8.0, 0.0),
            ignore_sphere: false,
            hit: Some(Hit {
                sphere: true,
                fraction: 0.5,
                point: Vec3::new(0.0, 6.0, 0.0),
            }),
        },
        Ray {
            name: "short of the sphere",
            origin: Vec3::new(0.0, 10.0, 0.0),
            direction: Vec3::new(0.0, -2.0, 0.0),
            ignore_sphere: false,
            hit: None,
        },
        Ray {
            name: "from inside the sphere",
            origin: Vec3::new(0.0, 5.0, 0.0),
            direction: Vec3::new(0.0, -10.0, 0.0),
            ignore_sphere: false,
            hit: Some(Hit {
                sphere: true,
                fraction: 0.0,
                point: Vec3::new(0.0, 5.0, 0.0),
            }),
        },
        Ray {
            name: "from inside the ignored sphere",
            origin: Vec3::new(0.0, 5.0, 0.0),
            direction: Vec3::new(0.0, -10.0, 0.0),
            ignore_sphere: true,
            hit: Some(Hit {
                sphere: false,
                fraction: 0.4,
                point: Vec3::new(0.0, 1.0, 0.0),
            }),
        },
    ];
    for ray in rays {
        let ignore = [sphere_body];
        let filter = if ray.ignore_sphere {
            QueryFilter { ignore: &ignore }
        } else {
            QueryFilter::default()
        };
        let hit = state.cast_ray(ray.origin, ray.direction, filter);
        match (hit, ray.hit) {
            (None, None) => {}
            (Some(hit), Some(expected)) => {
                let entity = if expected.sphere { sphere } else { floor };
                assert_eq!(hit.entity, Some(entity), "{}", ray.name);
                assert!(
                    (hit.fraction - expected.fraction).abs() < 0.01,
                    "{}: fraction {}",
                    ray.name,
                    hit.fraction
                );
                if expected.fraction > 0.0 {
                    assert!(
                        hit.point.distance(expected.point) < 0.05,
                        "{}: point {}",
                        ray.name,
                        hit.point
                    );
                    assert!(
                        hit.normal.dot(Vec3::Y) > 0.99,
                        "{}: normal {}",
                        ray.name,
                        hit.normal
                    );
                }
            }
            (hit, _) => panic!("{}: got {hit:?}", ray.name),
        }
    }
}
