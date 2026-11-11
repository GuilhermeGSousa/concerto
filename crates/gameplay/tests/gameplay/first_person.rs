use concerto_ecs::system::executor::single_thread::SingleThreadedExecutor;
use concerto_ecs::system::schedule::Schedule;
use concerto_ecs::{Entity, World};
use concerto_foundation::time::Time;
use concerto_foundation::transform::Transform;
use concerto_gameplay::actions::bind_gameplay_actions;
use concerto_gameplay::first_person::{
    FirstPersonView, FirstPersonWalker, place_first_person_views, read_first_person_input,
    spawn_first_person_walker, step_first_person_walkers,
};
use concerto_physics::body::BodyId;
use concerto_physics::collider::{Collider, register_colliders};
use concerto_physics::physics_pipeline::PhysicsPipeline;
use concerto_physics::physics_state::PhysicsState;
use concerto_window::input::actions::ActionMap;
use concerto_window::input::{
    GamepadAxis, GamepadId, Input, JoinSettings, PlayerDevice, PlayerIndex, Players,
};
use glam::{Vec2, Vec3};

const STEP_RISE: f32 = 0.18;
const STEP_RUN: f32 = 0.3;

fn world() -> World {
    let mut world = World::new();
    world.register_component::<Collider>();
    world.register_component::<Transform>();
    world.insert_resource(PhysicsState::new());
    world.insert_resource(Time::new());
    world.spawn((
        Collider::cuboid(50.0, 0.5, 50.0),
        Transform::from_translation(Vec3::new(0.0, -0.5, 0.0)),
    ));
    world
}

fn run<M>(world: &mut World, system: impl concerto_ecs::IntoSystem<(), M> + 'static) {
    let mut schedule = Schedule::new();
    schedule.add_system(system);
    schedule.compile::<SingleThreadedExecutor>(world).run(world);
}

fn walker(world: &mut World, player: u8, at: Vec3) -> Entity {
    let mut spawned = None;
    run(world, move |mut cmd: concerto_ecs::CommandQueue| {
        spawn_first_person_walker(
            &mut cmd,
            PlayerIndex::new(player),
            Transform::from_translation(at),
            FirstPersonWalker::default(),
        );
    });
    let mut walkers = world.query::<(Entity, &PlayerIndex), ()>();
    for (entity, index) in walkers.iter(world) {
        if index.index() == player {
            spawned = Some(entity);
        }
    }
    spawned.unwrap()
}

fn simulate(world: &mut World, seconds: f32) {
    run(world, register_colliders);
    let mut schedule = Schedule::new();
    schedule.add_system(step_first_person_walkers);
    let mut drive = schedule.compile::<SingleThreadedExecutor>(world);
    let mut pipeline = PhysicsPipeline::new();
    let steps = (seconds / Time::fixed_delta_time().as_secs_f32()).round() as u32;
    for _ in 0..steps {
        drive.run(world);
        pipeline.step(world.get_resource_mut::<PhysicsState>().unwrap());
    }
    let mut bodies = world.query::<(Entity, &BodyId), ()>();
    let bodies: Vec<Entity> = bodies.iter(world).map(|(entity, _)| entity).collect();
    for entity in bodies {
        let body = *world.get_component_for_entity::<BodyId>(entity).unwrap();
        let translation = world
            .get_resource::<PhysicsState>()
            .unwrap()
            .body_transform(body)
            .translation;
        world
            .get_component_for_entity_mut::<Transform>(entity)
            .unwrap()
            .translation = translation;
    }
}

fn steer(world: &mut World, walker: Entity, push: Vec2) {
    world
        .get_component_for_entity_mut::<FirstPersonWalker>(walker)
        .unwrap()
        .steer(push);
}

fn position(world: &World, entity: Entity) -> Vec3 {
    world
        .get_component_for_entity::<Transform>(entity)
        .unwrap()
        .translation
}

fn stairs(world: &mut World, first_step_z: f32, steps: u32) {
    for step in 0..steps {
        let top = STEP_RISE * (step + 1) as f32;
        let near = first_step_z - STEP_RUN * step as f32;
        world.spawn((
            Collider::cuboid(2.0, top / 2.0, 5.0),
            Transform::from_translation(Vec3::new(0.0, top / 2.0, near - 5.0)),
        ));
    }
}

#[test]
fn a_walker_stands_on_the_floor_and_walks_then_hurries_where_it_faces() {
    let mut world = world();
    let walker = walker(&mut world, 0, Vec3::new(0.0, 0.5, 0.0));
    let settings = FirstPersonWalker::default();

    simulate(&mut world, 1.0);
    let rest = position(&world, walker);
    assert!(rest.y.abs() < 0.03, "feet rest on the floor, at {rest}");
    assert!(
        rest.with_y(0.0).length() < 0.01,
        "it stands still, at {rest}"
    );

    steer(&mut world, walker, Vec2::new(0.0, settings.hurry_threshold));
    simulate(&mut world, 2.0);
    let walked = position(&world, walker);
    let distance = rest.z - walked.z;
    assert!(
        (distance - settings.walk_speed * 2.0).abs() < 0.1,
        "a push at the hurry threshold walks forward, along -Z, at walk speed: {distance} m in 2 s"
    );
    assert!(walked.x.abs() < 0.01 && walked.y.abs() < 0.03, "{walked}");

    steer(&mut world, walker, Vec2::new(0.0, 1.0));
    simulate(&mut world, 2.0);
    let hurried = walked.z - position(&world, walker).z;
    assert!(
        (hurried - settings.hurry_speed * 2.0).abs() < 0.1,
        "a full push hurries: {hurried} m in 2 s"
    );

    world
        .get_component_for_entity_mut::<FirstPersonWalker>(walker)
        .unwrap()
        .look(std::f32::consts::FRAC_PI_2, 0.0);
    let before = position(&world, walker);
    simulate(&mut world, 1.0);
    let turned = position(&world, walker) - before;
    assert!(
        turned.x < -settings.hurry_speed * 0.9 && turned.z.abs() < 0.05,
        "after a quarter turn left, forward is -X: moved {turned}"
    );
}

#[test]
fn a_walker_climbs_stairs_and_a_wall_stops_it() {
    let mut world = world();
    stairs(&mut world, -1.0, 5);
    let climber = walker(&mut world, 0, Vec3::ZERO);
    steer(&mut world, climber, Vec2::new(0.0, 1.0));

    simulate(&mut world, 3.0);

    let top = position(&world, climber);
    assert!(
        (top.y - STEP_RISE * 5.0).abs() < 0.05 && top.z < -1.0 - STEP_RUN * 5.0,
        "the walker stands on the top step, at {top}"
    );

    let mut world = self::world();
    world.spawn((
        Collider::cuboid(5.0, 1.5, 0.1),
        Transform::from_translation(Vec3::new(0.0, 1.5, -2.1)),
    ));
    let blocked = walker(&mut world, 0, Vec3::ZERO);
    let radius = FirstPersonWalker::default().radius;
    steer(&mut world, blocked, Vec2::new(0.0, 1.0));

    simulate(&mut world, 3.0);

    let stopped = position(&world, blocked);
    assert!(
        (stopped.z - (-2.0 + radius)).abs() < 0.05 && stopped.y.abs() < 0.03,
        "the wall at z = -2 holds the walker a radius away, at {stopped}"
    );
}

#[test]
fn each_walker_reads_its_own_players_stick_and_its_view_sits_at_its_eyes() {
    let mut world = world();
    world.register_component::<FirstPersonView>();
    let mut input = Input::default();
    let pads = [GamepadId::new(0), GamepadId::new(1)];
    let mut players = Players::new(JoinSettings::new().with_max_players(2));
    for (index, pad) in pads.into_iter().enumerate() {
        input.connect_gamepad(pad, "Test pad");
        players
            .assign(PlayerIndex::new(index as u8), PlayerDevice::Gamepad(pad))
            .unwrap();
    }
    input.update_gamepad_axis(pads[1], GamepadAxis::LeftStickY, 1.0);
    let mut actions = ActionMap::default();
    bind_gameplay_actions(&mut actions);
    world.insert_resource(input);
    world.insert_resource(players);
    world.insert_resource(actions);
    let first = walker(&mut world, 0, Vec3::new(-2.0, 0.0, 0.0));
    let second = walker(&mut world, 1, Vec3::new(2.0, 0.0, 0.0));
    let view = world.spawn((FirstPersonView { walker: second }, Transform::default()));

    run(&mut world, read_first_person_input);
    simulate(&mut world, 1.0);
    run(&mut world, place_first_person_views);

    let still = position(&world, first);
    let moved = position(&world, second);
    assert!(
        (still - Vec3::new(-2.0, 0.0, 0.0)).length() < 0.03,
        "player one's walker stays, at {still}"
    );
    assert!(moved.z < -3.0, "player two's walker hurries, to {moved}");
    let eye_height = FirstPersonWalker::default().eye_height;
    let eyes = position(&world, view);
    assert!(
        (eyes - (moved + Vec3::Y * eye_height)).length() < 1e-4,
        "the view sits at the walker's eyes: {eyes} against {moved}"
    );
}
