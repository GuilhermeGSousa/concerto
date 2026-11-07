use concerto_director::{Lens, VirtualCamera};
use concerto_ecs::entity::hierarchy::{ChildOf, Children};
use concerto_ecs::system::executor::single_thread::SingleThreadedExecutor;
use concerto_ecs::system::schedule::Schedule;
use concerto_ecs::{Changed, Entity, IntoSystemConfig, Query, ResMut, Resource, World};
use concerto_foundation::time::Time;
use concerto_foundation::transform::Transform;
use concerto_gameplay::camera::{CameraBoom, CameraPivot, CameraSettings, move_camera_pivot};
use concerto_gameplay::third_person::{
    CameraStance, ShoulderSide, ThirdPersonCamera, update_third_person_cameras,
};
use concerto_window::input::Input;
use glam::{EulerRot, Vec3};

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
    world.insert_resource(CameraSettings::default());
    world
}

fn rig(world: &mut World, rig: ThirdPersonCamera) -> Rig {
    let pivot = world.spawn((CameraPivot::default(), Transform::default(), rig));
    let camera = world.spawn((
        ChildOf::new(pivot),
        Transform::from_translation(CameraBoom::default().offset),
        CameraBoom::default(),
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
fn aiming_rig_writes_its_cameras_boom_and_lens() {
    let mut world = world();
    let rig = rig(
        &mut world,
        ThirdPersonCamera::default().with_stance(CameraStance::Aim),
    );

    run_rigs(&mut world);

    let boom = boom(&world, rig.camera);
    assert_vec_near(boom.offset, Vec3::NEG_Z * 1.2);
    assert_vec_near(boom.shoulder, Vec3::NEG_X * 0.5);
    let lens = lens(&world, rig.camera);
    assert_near(lens.fovy, 35f32.to_radians());
    assert_eq!(lens.znear, 0.5);
    assert_eq!(lens.zfar, 50.0);
}

#[test]
fn left_side_puts_the_shoulder_on_the_views_left() {
    let mut world = world();
    let rig = rig(
        &mut world,
        ThirdPersonCamera::default()
            .with_stance(CameraStance::Aim)
            .with_side(ShoulderSide::Left),
    );

    run_rigs(&mut world);

    assert_vec_near(boom(&world, rig.camera).shoulder, Vec3::X * 0.5);
}

#[derive(Resource, Default)]
struct ChangedCounts {
    booms: usize,
    cameras: usize,
    rigs: usize,
}

fn count_changed(
    booms: Query<Entity, Changed<CameraBoom>>,
    cameras: Query<Entity, Changed<VirtualCamera>>,
    rigs: Query<Entity, Changed<ThirdPersonCamera>>,
    mut counts: ResMut<ChangedCounts>,
) {
    counts.booms = booms.iter().count();
    counts.cameras = cameras.iter().count();
    counts.rigs = rigs.iter().count();
}

#[test]
fn settled_rig_writes_nothing() {
    let mut world = world();
    world.insert_resource(ChangedCounts::default());
    rig(
        &mut world,
        ThirdPersonCamera::default().with_stance(CameraStance::Aim),
    );
    let mut schedule = Schedule::new();
    schedule.add_system(update_third_person_cameras);
    schedule.add_system(count_changed.after(update_third_person_cameras));
    let mut compiled = schedule.compile::<SingleThreadedExecutor>(&mut world);

    compiled.run(&mut world);
    compiled.run(&mut world);

    let counts = world.get_resource::<ChangedCounts>().unwrap();
    assert_eq!(counts.booms, 0, "the boom was stamped changed");
    assert_eq!(counts.cameras, 0, "the camera was stamped changed");
    assert_eq!(counts.rigs, 0, "the rig was stamped changed");
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
