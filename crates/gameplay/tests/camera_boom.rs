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

fn rig(world: &mut World, target_at: Vec3) -> Entity {
    rig_with(
        world,
        target_at,
        Transform::from_translation(PIVOT),
        CameraBoom::default(),
    )
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

fn wall(world: &mut World, near_face_z: f32, half_thickness: f32) {
    world.spawn((
        Collider::cuboid(5.0, 5.0, half_thickness),
        Transform::from_translation(PIVOT + Vec3::new(0.0, 0.0, near_face_z - half_thickness)),
    ));
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

fn assert_near(actual: Vec3, expected: Vec3) {
    assert!(
        (actual - expected).length() < 0.01,
        "expected the camera at {expected:?}, got {actual:?}"
    );
}

#[test]
fn camera_with_nothing_in_the_way_sits_at_its_offset() {
    let mut world = world();
    let camera = rig(&mut world, Vec3::new(10.0, 1.0, 0.0));

    let translation = run_boom(&mut world, camera);

    assert_near(translation, CameraBoom::default().offset);
}

#[test]
fn wall_between_pivot_and_camera_pulls_it_in_to_the_probe_radius() {
    let mut world = world();
    let camera = rig(&mut world, Vec3::new(10.0, 1.0, 0.0));
    wall(&mut world, -1.9, 0.1);

    let translation = run_boom(&mut world, camera);

    let boom = CameraBoom::default();
    assert_near(translation, Vec3::NEG_Z * (1.9 - boom.probe_radius));
}

#[test]
fn followed_capsule_around_the_pivot_does_not_pull_the_camera_in() {
    let mut world = world();
    let camera = rig(&mut world, PIVOT);

    let translation = run_boom(&mut world, camera);

    assert_near(translation, CameraBoom::default().offset);
}

#[test]
fn wall_closer_than_min_distance_holds_the_camera_at_min_distance() {
    let mut world = world();
    let camera = rig(&mut world, Vec3::new(10.0, 1.0, 0.0));
    wall(&mut world, -0.35, 0.05);

    let translation = run_boom(&mut world, camera);

    let boom = CameraBoom::default();
    assert_near(translation, Vec3::NEG_Z * boom.min_distance);
}

#[test]
fn sideways_shoulder_with_nothing_in_the_way_puts_the_camera_at_shoulder_plus_offset() {
    let mut world = world();
    let boom = CameraBoom::default().with_shoulder(Vec3::X * 0.5);
    let camera = rig_with(
        &mut world,
        Vec3::new(10.0, 1.0, 0.0),
        Transform::from_translation(PIVOT),
        boom.clone(),
    );

    let translation = run_boom(&mut world, camera);

    assert_near(translation, boom.shoulder + boom.offset);
}

#[test]
fn wall_beside_the_pivot_stops_the_shoulder_short_and_keeps_the_full_offset() {
    let mut world = world();
    let boom = CameraBoom::default().with_shoulder(Vec3::X * 1.0);
    let camera = rig_with(
        &mut world,
        Vec3::new(-10.0, 1.0, 0.0),
        Transform::from_translation(PIVOT),
        boom.clone(),
    );
    let near_face_x = 0.6;
    let half_thickness = 0.1;
    world.spawn((
        Collider::cuboid(half_thickness, 5.0, 5.0),
        Transform::from_translation(PIVOT + Vec3::new(near_face_x + half_thickness, 0.0, 0.0)),
    ));

    let translation = run_boom(&mut world, camera);

    let shoulder = Vec3::X * (near_face_x - boom.probe_radius - BOOM_SKIN);
    assert_near(translation, shoulder + boom.offset);
}

#[test]
fn shoulder_stays_level_when_the_pivot_pitches() {
    let mut world = world();
    let boom = CameraBoom::default().with_shoulder(Vec3::Y * -0.5);
    let rotation = Quat::from_rotation_x(30f32.to_radians());
    let camera = rig_with(
        &mut world,
        Vec3::new(10.0, 1.0, 0.0),
        Transform::from_translation_rotation(PIVOT, rotation),
        boom.clone(),
    );

    let translation = run_boom(&mut world, camera);

    assert_near(
        translation,
        rotation.inverse() * (Vec3::Y * -0.5) + boom.offset,
    );
}

#[test]
fn shoulder_turns_with_the_pivots_yaw() {
    let mut world = world();
    let boom = CameraBoom::default().with_shoulder(Vec3::X * 0.5);
    let rotation = Quat::from_rotation_y(std::f32::consts::FRAC_PI_2);
    let camera = rig_with(
        &mut world,
        Vec3::new(10.0, 1.0, 10.0),
        Transform::from_translation_rotation(PIVOT, rotation),
        boom.clone(),
    );

    let translation = run_boom(&mut world, camera);

    assert_near(translation, boom.shoulder + boom.offset);
}
