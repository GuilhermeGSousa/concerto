use std::sync::Arc;

use crate::common::{physics_world, register_bodies};
use concerto_ecs::entity::Entity;
use concerto_ecs::world::World;
use concerto_foundation::transform::Transform;
use concerto_mesh::primitives::plane;
use glam::{Quat, Vec2, Vec3};

use concerto_physics::body::BodyId;
use concerto_physics::collider::{Collider, ColliderOffset};
use concerto_physics::physics_pipeline::PhysicsPipeline;
use concerto_physics::physics_state::PhysicsState;
use concerto_physics::rigid_body::{AllowedDofs, RigidBody};
use concerto_physics::shape::PhysicsShape;

struct Landing {
    name: &'static str,
    floor: fn(&mut World),
    body: fn(&mut World) -> Entity,
    rest: f32,
    upright: bool,
}

fn floor_box(world: &mut World) {
    world.spawn((Collider::cuboid(100.0, 1.0, 100.0), Transform::default()));
}

fn dropped(world: &mut World, collider: Collider, height: f32) -> Entity {
    world.spawn((
        RigidBody::default(),
        collider,
        Transform::from_translation(Vec3::new(0.0, height, 0.0)),
    ))
}

fn upright_capsule(world: &mut World, offset: bool) -> Entity {
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

#[test]
fn a_body_dropped_onto_a_collider_comes_to_rest_on_top_of_it() {
    let landings = [
        Landing {
            name: "a sphere on a box",
            floor: floor_box,
            body: |world| dropped(world, Collider::sphere(1.0), 10.0),
            rest: 2.0,
            upright: false,
        },
        Landing {
            name: "a rotation-locked capsule",
            floor: floor_box,
            body: |world| upright_capsule(world, false),
            rest: 2.0,
            upright: true,
        },
        Landing {
            name: "a bottom-origin capsule",
            floor: floor_box,
            body: |world| upright_capsule(world, true),
            rest: 1.0,
            upright: true,
        },
        Landing {
            name: "a sphere on a mesh",
            floor: |world| {
                let shape = PhysicsShape::from_mesh(&plane(Vec2::splat(40.0))).unwrap();
                world.spawn((Collider::from_shape(Arc::new(shape)), Transform::default()));
            },
            body: |world| dropped(world, Collider::sphere(0.5), 4.0),
            rest: 0.5,
            upright: false,
        },
        Landing {
            name: "a sphere on a scaled box",
            floor: |world| {
                world.spawn((
                    Collider::cuboid(1.0, 1.0, 1.0),
                    Transform::from_translation(Vec3::new(0.0, -1.0, 0.0))
                        .with_scale(Vec3::splat(5.0)),
                ));
            },
            body: |world| dropped(world, Collider::sphere(0.5), 8.0),
            rest: 4.5,
            upright: false,
        },
    ];
    for landing in landings {
        let mut world = physics_world();
        let mut pipeline = PhysicsPipeline::new();
        (landing.floor)(&mut world);
        let entity = (landing.body)(&mut world);
        register_bodies(&mut world);

        for _ in 0..240 {
            pipeline.step(world.get_resource_mut::<PhysicsState>().unwrap());
        }

        let body = *world.get_component_for_entity::<BodyId>(entity).unwrap();
        let transform = world
            .get_resource::<PhysicsState>()
            .unwrap()
            .body_transform(body);
        assert!(
            (transform.translation.y - landing.rest).abs() < 0.1,
            "{}: expected to rest at y = {}, was {}",
            landing.name,
            landing.rest,
            transform.translation.y
        );
        if landing.upright {
            assert!(
                transform.rotation.dot(Quat::IDENTITY).abs() > 0.999,
                "{}: {:?}",
                landing.name,
                transform.rotation
            );
        }
    }
}
