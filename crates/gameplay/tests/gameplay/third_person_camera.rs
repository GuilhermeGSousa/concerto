use concerto_director::{Lens, VirtualCamera};
use concerto_ecs::entity::hierarchy::{ChildOf, Children};
use concerto_ecs::system::executor::single_thread::SingleThreadedExecutor;
use concerto_ecs::system::schedule::Schedule;
use concerto_ecs::{Entity, IntoSystemConfig, World};
use concerto_foundation::time::Time;
use concerto_foundation::transform::Transform;
use concerto_gameplay::actions::bind_gameplay_actions;
use concerto_gameplay::camera::{CameraBoom, CameraPivot, CameraSettings, move_camera_pivot};
use concerto_gameplay::third_person::{
    CameraStance, ShoulderSide, ThirdPersonCamera, update_third_person_cameras,
};
use concerto_window::input::actions::ActionMap;
use concerto_window::input::{GamepadAxis, GamepadId, Input};
use glam::{EulerRot, Vec3};
use std::time::Duration;

const LARGE_MOUSE_DELTA_X: f64 = 100.0;
const LARGE_MOUSE_DELTA_Y: f64 = -100_000.0;

struct Rig {
    pivot: Entity,
    camera: Entity,
}

fn world() -> World {
    let mut world = World::new();
    world.register_component::<Transform>();
    world.register_component::<ChildOf>();
    world.register_component::<Children>();
    world.insert_resource(Time::new());
    world.insert_resource(Input::new());
    let mut actions = ActionMap::default();
    bind_gameplay_actions(&mut actions);
    world.insert_resource(actions);
    world.insert_resource(CameraSettings::default());
    world
}

fn rig(world: &mut World, rig: ThirdPersonCamera) -> Rig {
    let boom = CameraBoom::default();
    let pivot = world.spawn((CameraPivot::default(), Transform::default(), rig));
    let camera = world.spawn((
        ChildOf::new(pivot),
        Transform::from_translation(boom.offset),
        boom,
        VirtualCamera::new(0).with_lens(Lens {
            fovy: 1.0,
            znear: 0.5,
            zfar: 50.0,
        }),
    ));
    Rig { pivot, camera }
}

fn run_rigs(world: &mut World) {
    let mut schedule = Schedule::new();
    schedule.add_system(update_third_person_cameras);
    schedule.compile::<SingleThreadedExecutor>(world).run(world);
}

fn run_rigs_and_pivots(world: &mut World) {
    let mut schedule = Schedule::new();
    schedule.add_system(update_third_person_cameras);
    schedule.add_system(move_camera_pivot.after(update_third_person_cameras));
    schedule.compile::<SingleThreadedExecutor>(world).run(world);
}

fn run_pivots(world: &mut World) {
    let mut schedule = Schedule::new();
    schedule.add_system(move_camera_pivot);
    schedule.compile::<SingleThreadedExecutor>(world).run(world);
}

fn boom(world: &World, camera: Entity) -> &CameraBoom {
    world
        .get_component_for_entity::<CameraBoom>(camera)
        .unwrap()
}

fn lens(world: &World, camera: Entity) -> Lens {
    world
        .get_component_for_entity::<VirtualCamera>(camera)
        .unwrap()
        .lens
        .expect("the rig gives its camera a lens")
}

fn pitch(world: &World, pivot: Entity) -> f32 {
    world
        .get_component_for_entity::<Transform>(pivot)
        .unwrap()
        .rotation
        .to_euler(EulerRot::YXZ)
        .1
}

fn yaw(world: &World, pivot: Entity) -> f32 {
    world
        .get_component_for_entity::<CameraPivot>(pivot)
        .unwrap()
        .yaw()
}

fn assert_vec_near(actual: Vec3, expected: Vec3) {
    assert!(
        (actual - expected).length() < 1e-5,
        "expected {expected:?}, got {actual:?}"
    );
}

fn assert_near(actual: f32, expected: f32) {
    assert!(
        (actual - expected).abs() < 1e-4,
        "expected {expected}, got {actual}"
    );
}

#[test]
fn an_aiming_rig_writes_its_cameras_boom_on_its_side_and_its_lens() {
    let mut world = world();
    let right = rig(
        &mut world,
        ThirdPersonCamera::default().with_stance(CameraStance::Aim),
    );
    let left = rig(
        &mut world,
        ThirdPersonCamera::default()
            .with_stance(CameraStance::Aim)
            .with_side(ShoulderSide::Left),
    );

    run_rigs(&mut world);

    let boom_of_right = boom(&world, right.camera);
    assert_vec_near(boom_of_right.offset, Vec3::NEG_Z * 1.2);
    assert_vec_near(boom_of_right.shoulder, Vec3::NEG_X * 0.5);
    assert_vec_near(boom(&world, left.camera).shoulder, Vec3::X * 0.5);
    let lens = lens(&world, right.camera);
    assert_near(lens.fovy, 35f32.to_radians());
    assert_eq!(lens.znear, 0.5);
    assert_eq!(lens.zfar, 50.0);
}

#[test]
fn aiming_rig_widens_the_pitch_limit_and_slows_the_look() {
    let mut world = world();
    let aiming = rig(
        &mut world,
        ThirdPersonCamera::default().with_stance(CameraStance::Aim),
    );
    let plain = world.spawn((CameraPivot::default(), Transform::default()));
    world
        .get_resource_mut::<Input>()
        .unwrap()
        .update_mouse_delta((LARGE_MOUSE_DELTA_X, LARGE_MOUSE_DELTA_Y));

    run_rigs_and_pivots(&mut world);

    assert_near(pitch(&world, aiming.pivot), 60f32.to_radians());
    assert_near(pitch(&world, plain), 30f32.to_radians());
    assert_near(yaw(&world, aiming.pivot), yaw(&world, plain) * 0.5);
    assert!(yaw(&world, plain).abs() > 0.01);
}

#[test]
fn leaving_aim_eases_the_pitch_back_into_range() {
    let mut world = world();
    let rig = rig(
        &mut world,
        ThirdPersonCamera::default().with_stance(CameraStance::Aim),
    );
    world
        .get_resource_mut::<Input>()
        .unwrap()
        .update_mouse_delta((0.0, LARGE_MOUSE_DELTA_Y));
    run_rigs_and_pivots(&mut world);
    assert_near(pitch(&world, rig.pivot), 60f32.to_radians());
    world.get_resource_mut::<Input>().unwrap().update();

    let camera = world
        .get_component_for_entity_mut::<ThirdPersonCamera>(rig.pivot)
        .unwrap();
    camera.stance = CameraStance::Follow;
    camera.step(0.2);
    run_pivots(&mut world);

    assert_near(pitch(&world, rig.pivot), 45f32.to_radians());
}

#[test]
fn full_right_stick_turns_the_pivot_by_the_stick_turn_rate() {
    let mut world = world();
    let pivot = world.spawn((CameraPivot::default(), Transform::default()));
    let pad = GamepadId::new(0);
    let input = world.get_resource_mut::<Input>().unwrap();
    input.connect_gamepad(pad, "pad");
    input.update_gamepad_axis(pad, GamepadAxis::RightStickX, 1.0);
    std::thread::sleep(Duration::from_millis(5));
    world.get_resource_mut::<Time>().unwrap().update();
    let dt = world.get_resource::<Time>().unwrap().delta().as_secs_f32();
    let rate = world
        .get_resource::<CameraSettings>()
        .unwrap()
        .stick_turn_rate;

    run_pivots(&mut world);

    assert!(dt > 0.0);
    assert_near(yaw(&world, pivot), -rate * dt);
    assert_near(pitch(&world, pivot), 0.0);
}
