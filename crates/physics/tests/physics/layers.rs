use crate::common::physics_world;
use concerto_ecs::entity::Entity;
use concerto_ecs::system::executor::single_thread::SingleThreadedExecutor;
use concerto_ecs::system::schedule::Schedule;
use concerto_ecs::world::World;
use concerto_ecs::IntoSystemConfig;
use concerto_foundation::transform::Transform;
use glam::Vec3;

use concerto_physics::body::BodyId;
use concerto_physics::collider::{register_colliders, Collider};
use concerto_physics::layers::{apply_collision_layers, CollisionLayers, Layers};
use concerto_physics::physics_pipeline::PhysicsPipeline;
use concerto_physics::physics_state::PhysicsState;
use concerto_physics::query::QueryFilter;
use concerto_physics::rigid_body::RigidBody;
use concerto_physics::step_simulation;

const PLATFORM: Layers = Layers::bit(Layers::FIRST_FREE);

fn height(world: &World, entity: Entity) -> f32 {
    let body = *world.get_component_for_entity::<BodyId>(entity).unwrap();
    world
        .get_resource::<PhysicsState>()
        .unwrap()
        .body_transform(body)
        .translation
        .y
}

#[test]
fn bodies_collide_and_queries_see_by_collision_layer() {
    let mut world = physics_world();
    world.register_component::<CollisionLayers>();
    world.insert_resource(PhysicsPipeline::new());
    let floor = world.spawn((
        Collider::cuboid(20.0, 0.5, 20.0),
        Transform::from_translation(Vec3::new(0.0, -0.5, 0.0)),
    ));
    let platform = world.spawn((
        Collider::cuboid(5.0, 0.25, 5.0),
        CollisionLayers::new(PLATFORM, Layers::ALL),
        Transform::from_translation(Vec3::new(0.0, 2.0, 0.0)),
    ));
    let ball = |world: &mut World, x: f32, layers: Option<CollisionLayers>| {
        let entity = world.spawn((
            RigidBody::default(),
            Collider::sphere(0.25),
            Transform::from_translation(Vec3::new(x, 4.0, 0.0)),
        ));
        if let Some(layers) = layers {
            world.insert(layers, entity);
        }
        entity
    };
    let stopped = ball(&mut world, -2.0, None);
    let through = ball(
        &mut world,
        0.0,
        Some(CollisionLayers::new(
            Layers::DEFAULT,
            Layers::ALL.without(PLATFORM),
        )),
    );
    let unseen = ball(
        &mut world,
        2.0,
        Some(CollisionLayers::new(Layers::bit(5), Layers::DEFAULT)),
    );
    let mut schedule = Schedule::new();
    schedule.add_system((register_colliders, apply_collision_layers, step_simulation).chain());
    let mut schedule = schedule.compile::<SingleThreadedExecutor>(&mut world);
    for _ in 0..90 {
        schedule.run(&mut world);
    }
    struct Rest {
        name: &'static str,
        ball: Entity,
        height: f32,
    }
    let rests = [
        Rest {
            name: "a default ball lands on the platform",
            ball: stopped,
            height: 2.5,
        },
        Rest {
            name: "a ball that does not collide with the platform's layer passes it",
            ball: through,
            height: 0.25,
        },
        Rest {
            name: "a ball the platform does collide with, but that only collides with the default layer, passes it",
            ball: unseen,
            height: 0.25,
        },
    ];
    for rest in rests {
        let y = height(&world, rest.ball);
        assert!(
            (y - rest.height).abs() < 0.05,
            "{}: was at y = {y}",
            rest.name
        );
    }

    struct Ray {
        name: &'static str,
        layers: Layers,
        hit: Option<Entity>,
    }
    let rays = [
        Ray {
            name: "the default query sees every layer but ragdolls",
            layers: QueryFilter::DEFAULT_LAYERS,
            hit: Some(platform),
        },
        Ray {
            name: "a query for the default layer passes the platform",
            layers: Layers::DEFAULT,
            hit: Some(floor),
        },
        Ray {
            name: "a query for the platform's layer sees it",
            layers: PLATFORM,
            hit: Some(platform),
        },
        Ray {
            name: "a query for no layer sees nothing",
            layers: Layers::NONE,
            hit: None,
        },
    ];
    let state = world.get_resource::<PhysicsState>().unwrap();
    for ray in rays {
        let hit = state.cast_ray(
            Vec3::new(4.0, 10.0, 4.0),
            Vec3::new(0.0, -20.0, 0.0),
            QueryFilter::default().with_layers(ray.layers),
        );
        assert_eq!(hit.and_then(|hit| hit.entity), ray.hit, "{}", ray.name);
        let swept = state.cast_sphere(
            Vec3::new(4.0, 10.0, 4.0),
            0.1,
            Vec3::new(0.0, -20.0, 0.0),
            QueryFilter::default().with_layers(ray.layers),
        );
        assert_eq!(
            swept.and_then(|hit| hit.entity),
            ray.hit,
            "sweep: {}",
            ray.name
        );
    }

    world.insert(
        CollisionLayers::new(Layers::DEFAULT, Layers::ALL.without(PLATFORM)),
        stopped,
    );
    for _ in 0..60 {
        schedule.run(&mut world);
    }
    let y = height(&world, stopped);
    assert!(
        (y - 0.25).abs() < 0.05,
        "a ball whose layers change to pass the platform falls to the floor, was at y = {y}"
    );
}
