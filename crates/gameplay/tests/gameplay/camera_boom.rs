use concerto_ecs::entity::hierarchy::{ChildOf, Children};
use concerto_ecs::system::executor::single_thread::SingleThreadedExecutor;
use concerto_ecs::system::schedule::Schedule;
use concerto_ecs::{Entity, World};
use concerto_foundation::time::Time;
use concerto_foundation::transform::Transform;
use concerto_gameplay::camera::{CameraBoom, EntityFollow, update_camera_booms};
use concerto_physics::collider::{Collider, register_colliders};
use concerto_physics::physics_state::PhysicsState;
use glam::{Quat, Vec3};

const PIVOT: Vec3 = Vec3::new(0.0, 1.0, 0.0);
const BOOM_SKIN: f32 = 0.02;

fn world() -> World {
    let mut world = World::new();
    world.register_component::<Collider>();
    world.register_component::<Transform>();
    world.register_component::<ChildOf>();
    world.register_component::<Children>();
    world.insert_resource(PhysicsState::new());
    world.insert_resource(Time::new());
    world
}

fn rig_with(world: &mut World, target_at: Vec3, pivot: Transform, boom: CameraBoom) -> Entity {
    let target = world.spawn((
        Collider::capsule(0.5, 0.3),
        Transform::from_translation(target_at),
    ));
    let pivot = world.spawn((
        pivot,
        EntityFollow {
            target,
            offset: Vec3::ZERO,
        },
    ));
    world.spawn((
        ChildOf::new(pivot),
        Transform::from_translation(boom.offset),
        boom,
    ))
}

fn run_boom(world: &mut World, camera: Entity) -> Vec3 {
    let mut schedule = Schedule::new();
    schedule.add_system(register_colliders);
    schedule.compile::<SingleThreadedExecutor>(world).run(world);

    let mut schedule = Schedule::new();
    schedule.add_system(update_camera_booms);
    schedule.compile::<SingleThreadedExecutor>(world).run(world);

    world
        .get_component_for_entity::<Transform>(camera)
        .unwrap()
        .translation
}

struct Obstacle {
    center: Vec3,
    half_extents: Vec3,
}

struct Boom {
    name: &'static str,
    pivot: Quat,
    boom: CameraBoom,
    target_at: Vec3,
    obstacle: Option<Obstacle>,
    camera: Vec3,
}

#[test]
fn the_boom_holds_the_camera_at_its_shoulder_and_offset_pulled_in_by_what_is_in_the_way() {
    let boom = CameraBoom::default();
    let shoulder = |shoulder: Vec3| CameraBoom::default().with_shoulder(shoulder);
    let pitch = Quat::from_rotation_x(30f32.to_radians());
    let away = Vec3::new(10.0, 1.0, 10.0);
    let booms = [
        Boom {
            name: "the followed capsule around the pivot",
            pivot: Quat::IDENTITY,
            boom: boom.clone(),
            target_at: PIVOT,
            obstacle: None,
            camera: boom.offset,
        },
        Boom {
            name: "a wall behind the pivot",
            pivot: Quat::IDENTITY,
            boom: boom.clone(),
            target_at: away,
            obstacle: Some(Obstacle {
                center: PIVOT + Vec3::new(0.0, 0.0, -2.0),
                half_extents: Vec3::new(5.0, 5.0, 0.1),
            }),
            camera: Vec3::NEG_Z * (1.9 - boom.probe_radius),
        },
        Boom {
            name: "a wall closer than the min distance",
            pivot: Quat::IDENTITY,
            boom: boom.clone(),
            target_at: away,
            obstacle: Some(Obstacle {
                center: PIVOT + Vec3::new(0.0, 0.0, -0.4),
                half_extents: Vec3::new(5.0, 5.0, 0.05),
            }),
            camera: Vec3::NEG_Z * boom.min_distance,
        },
        Boom {
            name: "a wall beside the pivot",
            pivot: Quat::IDENTITY,
            boom: shoulder(Vec3::X),
            target_at: -away,
            obstacle: Some(Obstacle {
                center: PIVOT + Vec3::new(0.7, 0.0, 0.0),
                half_extents: Vec3::new(0.1, 5.0, 5.0),
            }),
            camera: Vec3::X * (0.6 - boom.probe_radius - BOOM_SKIN) + boom.offset,
        },
        Boom {
            name: "a pitched pivot",
            pivot: pitch,
            boom: shoulder(Vec3::Y * -0.5),
            target_at: away,
            obstacle: None,
            camera: pitch.inverse() * (Vec3::Y * -0.5) + boom.offset,
        },
        Boom {
            name: "a turned pivot",
            pivot: Quat::from_rotation_y(std::f32::consts::FRAC_PI_2),
            boom: shoulder(Vec3::X * 0.5),
            target_at: away,
            obstacle: None,
            camera: Vec3::X * 0.5 + boom.offset,
        },
    ];
    for case in booms {
        let mut world = world();
        if let Some(obstacle) = case.obstacle {
            let half = obstacle.half_extents;
            world.spawn((
                Collider::cuboid(half.x, half.y, half.z),
                Transform::from_translation(obstacle.center),
            ));
        }
        let camera = rig_with(
            &mut world,
            case.target_at,
            Transform::from_translation_rotation(PIVOT, case.pivot),
            case.boom,
        );

        let translation = run_boom(&mut world, camera);

        assert!(
            (translation - case.camera).length() < 0.01,
            "{}: expected the camera at {}, got {translation}",
            case.name,
            case.camera
        );
    }
}
