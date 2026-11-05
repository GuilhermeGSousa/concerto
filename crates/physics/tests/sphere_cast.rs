use std::sync::Arc;

use concerto_ecs::entity::Entity;
use concerto_ecs::world::World;
use concerto_foundation::transform::Transform;
use concerto_mesh::primitives::plane;
use glam::{Vec2, Vec3};
mod common;
use common::{physics_world, register_bodies};

use concerto_physics::body::BodyId;
use concerto_physics::collider::Collider;
use concerto_physics::physics_state::PhysicsState;
use concerto_physics::query::QueryFilter;
use concerto_physics::shape::PhysicsShape;

const RADIUS: f32 = 0.5;

fn at(translation: Vec3) -> Transform {
    Transform::from_translation_rotation(translation, Default::default())
}

fn world_with_wall() -> (World, Entity) {
    let mut world = physics_world();
    let wall = world.spawn((
        Collider::cuboid(0.5, 5.0, 5.0),
        at(Vec3::new(5.0, 0.0, 0.0)),
    ));
    register_bodies(&mut world);
    (world, wall)
}

fn body_of(world: &World, entity: Entity) -> BodyId {
    *world
        .get_component_for_entity::<BodyId>(entity)
        .expect("collider should have a BodyId")
}

#[test]
fn sphere_stops_radius_short_of_a_wall() {
    let (world, wall) = world_with_wall();
    let state = world.get_resource::<PhysicsState>().unwrap();

    let hit = state
        .cast_sphere(
            Vec3::ZERO,
            RADIUS,
            Vec3::new(10.0, 0.0, 0.0),
            QueryFilter::default(),
        )
        .expect("a sweep reaching the wall should hit it");

    assert_eq!(hit.entity, Some(wall));
    assert!(
        (hit.fraction - 0.4).abs() < 0.01,
        "expected fraction ~0.4, got {}",
        hit.fraction
    );
    assert!(
        (hit.point.x - 4.5).abs() < 0.01,
        "expected the point on the wall face at x = 4.5, got {:?}",
        hit.point
    );
    assert!(
        hit.normal.dot(Vec3::NEG_X) > 0.99,
        "expected the normal to face the sweep, got {:?}",
        hit.normal
    );
}

#[test]
fn sweep_ending_before_the_wall_misses() {
    let (world, _) = world_with_wall();
    let state = world.get_resource::<PhysicsState>().unwrap();

    let hit = state.cast_sphere(
        Vec3::ZERO,
        RADIUS,
        Vec3::new(3.0, 0.0, 0.0),
        QueryFilter::default(),
    );

    assert!(hit.is_none(), "a sweep stopping short should miss");
}

#[test]
fn sweep_starting_inside_a_body_hits_it_at_zero_unless_ignored() {
    let (mut world, wall) = world_with_wall();
    let blocker = world.spawn((
        Collider::cuboid(1.0, 1.0, 1.0),
        at(Vec3::new(0.5, 0.0, 0.0)),
    ));
    register_bodies(&mut world);
    let blocker_body = body_of(&world, blocker);
    let state = world.get_resource::<PhysicsState>().unwrap();

    for direction in [Vec3::new(10.0, 0.0, 0.0), Vec3::new(-10.0, 0.0, 0.0)] {
        let hit = state
            .cast_sphere(Vec3::ZERO, RADIUS, direction, QueryFilter::default())
            .expect("a sweep starting inside a body should hit it");
        assert_eq!(hit.entity, Some(blocker), "sweep along {direction:?}");
        assert!(
            hit.fraction.abs() < 1e-3,
            "expected fraction 0 along {direction:?}, got {}",
            hit.fraction
        );
    }

    let ignore = [blocker_body];
    let hit = state
        .cast_sphere(
            Vec3::ZERO,
            RADIUS,
            Vec3::new(10.0, 0.0, 0.0),
            QueryFilter { ignore: &ignore },
        )
        .expect("with the blocker ignored the sweep should reach the wall");
    assert_eq!(hit.entity, Some(wall));
    assert!(
        (hit.fraction - 0.4).abs() < 0.01,
        "expected fraction ~0.4, got {}",
        hit.fraction
    );
}

#[test]
fn sweep_hits_a_mesh_plane_from_either_side() {
    let mut world = physics_world();
    let shape = PhysicsShape::from_mesh(&plane(Vec2::splat(20.0))).expect("plane mesh shape");
    let floor = world.spawn((Collider::from_shape(Arc::new(shape)), at(Vec3::ZERO)));
    register_bodies(&mut world);
    let state = world.get_resource::<PhysicsState>().unwrap();

    for (origin, direction) in [
        (Vec3::new(0.0, 2.0, 0.0), Vec3::new(0.0, -4.0, 0.0)),
        (Vec3::new(0.0, -2.0, 0.0), Vec3::new(0.0, 4.0, 0.0)),
    ] {
        let hit = state
            .cast_sphere(origin, RADIUS, direction, QueryFilter::default())
            .unwrap_or_else(|| panic!("a sweep from {origin:?} should hit the plane"));
        assert_eq!(hit.entity, Some(floor));
        assert!(
            (hit.fraction - 0.375).abs() < 0.01,
            "expected fraction ~0.375 from {origin:?}, got {}",
            hit.fraction
        );
        assert!(
            hit.normal.dot(-direction.normalize()) > 0.99,
            "expected the normal to face the sweep from {origin:?}, got {:?}",
            hit.normal
        );
    }
}
