use std::f32::consts::FRAC_PI_2;

use concerto_app::{
    App,
    main_schedule::MainSchedulePlugin,
    plugins::{TimePlugin, TransformPlugin},
    schedule_groups::Update,
};
use concerto_ecs::{
    Entity, IntoSystemConfig, Query, World,
    component::scene::SceneEntityRef,
    events::{Event, event_channel::EventChannel, event_cursor::EventCursor},
};
use concerto_foundation::transform::Transform;
use concerto_name::Name;
use concerto_navigation::{
    NavAgent, NavArrived, NavDestination, NavLink, NavLinks, NavPoint, NavRoute, NavSteering,
    NavTarget, NavUnreachable, NavigationPlugin, steer_nav_agents,
};
use glam::{Quat, Vec3};

const DT: f32 = 0.05;

fn walk(agents: Query<(&NavSteering, &mut Transform)>) {
    for (steering, mut transform) in agents.iter() {
        transform.translation += steering.velocity * DT;
    }
}

fn app() -> App {
    let mut app = App::new();
    app.register_plugin(MainSchedulePlugin)
        .register_plugin(TimePlugin)
        .register_plugin(TransformPlugin)
        .register_plugin(NavigationPlugin);
    app.add_system(Update, walk.after(steer_nav_agents));
    app.finish_plugin_build();
    app
}

fn world(app: &mut App) -> &mut World {
    app.main_mut().world_mut()
}

fn spawn_point(app: &mut App, position: Vec3) -> Entity {
    spawn_facing_point(app, position, Quat::IDENTITY)
}

fn spawn_facing_point(app: &mut App, position: Vec3, rotation: Quat) -> Entity {
    world(app).spawn((
        Transform::from_translation(position).with_rotation(rotation),
        NavPoint::default(),
    ))
}

fn link(app: &mut App, from: Entity, to: &[Entity]) {
    world(app).insert(
        NavLinks {
            links: to
                .iter()
                .map(|&to| NavLink {
                    to: SceneEntityRef::Entity(to),
                    one_way: false,
                })
                .collect(),
        },
        from,
    );
}

fn spawn_agent(app: &mut App, position: Vec3) -> Entity {
    world(app).spawn((Transform::from_translation(position), NavAgent::new()))
}

fn settle(app: &mut App) {
    app.update();
    app.update();
}

fn events<T: Event + Clone + 'static>(app: &App) -> Vec<T> {
    app.get_resource::<EventChannel<T>>()
        .expect("the event is registered")
        .read_from(&mut EventCursor::default())
        .cloned()
        .collect()
}

fn steering(app: &mut App, agent: Entity) -> NavSteering {
    *world(app)
        .get_component_for_entity::<NavSteering>(agent)
        .expect("the agent has planned")
}

fn route(app: &mut App, agent: Entity) -> Option<NavRoute> {
    world(app)
        .get_component_for_entity::<NavRoute>(agent)
        .cloned()
}

fn has_destination(app: &mut App, agent: Entity) -> bool {
    world(app)
        .get_component_for_entity::<NavDestination>(agent)
        .is_some()
}

fn walk_until_arrived(app: &mut App, steps: usize) -> Option<NavArrived> {
    for _ in 0..steps {
        app.update();
        if let Some(arrived) = events::<NavArrived>(app).into_iter().next() {
            return Some(arrived);
        }
    }
    None
}

struct Corner {
    app: App,
    agent: Entity,
    a: Entity,
    b: Entity,
    c: Entity,
}

fn corner() -> Corner {
    let mut app = app();
    let a = spawn_point(&mut app, Vec3::ZERO);
    let b = spawn_point(&mut app, Vec3::new(0.0, 0.0, 4.0));
    let c = spawn_facing_point(
        &mut app,
        Vec3::new(4.0, 0.0, 4.0),
        Quat::from_rotation_y(FRAC_PI_2),
    );
    link(&mut app, a, &[b]);
    link(&mut app, b, &[c]);
    let agent = spawn_agent(&mut app, Vec3::new(0.0, 0.0, -0.5));
    settle(&mut app);
    Corner {
        app,
        agent,
        a,
        b,
        c,
    }
}

#[test]
fn an_agent_walks_to_its_destination_and_takes_the_points_facing() {
    let Corner {
        mut app, agent, c, ..
    } = corner();
    world(&mut app).insert(NavDestination::to_point(c), agent);

    let arrived = walk_until_arrived(&mut app, 600);

    assert_eq!(arrived, Some(NavArrived { agent, point: c }));
    let steering = steering(&mut app, agent);
    assert_eq!(steering.velocity, Vec3::ZERO);
    assert!(
        steering
            .facing
            .abs_diff_eq(Quat::from_rotation_y(FRAC_PI_2), 1e-5)
    );
    assert!(!has_destination(&mut app, agent));
    assert!(route(&mut app, agent).is_none());
    let position = world(&mut app)
        .get_component_for_entity::<Transform>(agent)
        .unwrap()
        .translation;
    assert!(position.distance(Vec3::new(4.0, 0.0, 4.0)) <= 0.25 + 1e-3);
}

#[test]
fn an_agent_reaches_a_point_named_by_its_anchor() {
    let Corner {
        mut app, agent, c, ..
    } = corner();
    world(&mut app).insert(
        NavPoint {
            anchor: Name::new("corner"),
            enabled: true,
        },
        c,
    );
    app.update();
    world(&mut app).insert(NavDestination::to_anchor(Name::new("corner")), agent);

    assert_eq!(
        walk_until_arrived(&mut app, 600),
        Some(NavArrived { agent, point: c })
    );
}

#[test]
fn an_unknown_anchor_is_unreachable() {
    let Corner { mut app, agent, .. } = corner();
    world(&mut app).insert(NavDestination::to_anchor(Name::new("nowhere")), agent);

    app.update();

    assert_eq!(
        events::<NavUnreachable>(&app),
        vec![NavUnreachable {
            agent,
            target: NavTarget::Anchor(Name::new("nowhere")),
        }]
    );
    assert!(!has_destination(&mut app, agent));
    assert!(route(&mut app, agent).is_none());
}

#[test]
fn disabling_a_point_on_the_route_replans_around_it() {
    let Corner {
        mut app,
        agent,
        a,
        b,
        c,
    } = corner();
    let d = spawn_point(&mut app, Vec3::new(4.0, 0.0, -1.0));
    link(&mut app, a, &[b, d]);
    link(&mut app, d, &[c]);
    settle(&mut app);
    world(&mut app).insert(NavDestination::to_point(c), agent);
    for _ in 0..20 {
        app.update();
    }
    assert_eq!(
        route(&mut app, agent).unwrap().remaining(),
        &[Vec3::new(0.0, 0.0, 4.0), Vec3::new(4.0, 0.0, 4.0)]
    );

    world(&mut app)
        .get_component_for_entity_mut::<NavPoint>(b)
        .unwrap()
        .enabled = false;
    app.update();
    app.update();

    let remaining = route(&mut app, agent).unwrap().remaining().to_vec();
    assert!(!remaining.contains(&Vec3::new(0.0, 0.0, 4.0)));
    assert_eq!(remaining.last(), Some(&Vec3::new(4.0, 0.0, 4.0)));
    assert_eq!(
        walk_until_arrived(&mut app, 600),
        Some(NavArrived { agent, point: c })
    );
}

#[test]
fn disabling_the_only_way_makes_the_destination_unreachable() {
    let Corner {
        mut app,
        agent,
        b,
        c,
        ..
    } = corner();
    world(&mut app).insert(NavDestination::to_point(c), agent);
    for _ in 0..20 {
        app.update();
    }
    assert!(steering(&mut app, agent).velocity != Vec3::ZERO);

    world(&mut app)
        .get_component_for_entity_mut::<NavPoint>(b)
        .unwrap()
        .enabled = false;
    app.update();
    app.update();

    assert_eq!(
        events::<NavUnreachable>(&app),
        vec![NavUnreachable {
            agent,
            target: NavTarget::Point(c),
        }]
    );
    assert!(!has_destination(&mut app, agent));
    assert!(route(&mut app, agent).is_none());
    assert_eq!(steering(&mut app, agent).velocity, Vec3::ZERO);
}

#[test]
fn speed_is_scaled_down_inside_the_slow_radius() {
    let mut app = app();
    let point = spawn_point(&mut app, Vec3::new(0.0, 0.0, 1.0));
    let agent = spawn_agent(&mut app, Vec3::ZERO);
    settle(&mut app);
    world(&mut app).insert(NavDestination::to_point(point), agent);

    app.update();

    let velocity = steering(&mut app, agent).velocity;
    assert!(velocity.abs_diff_eq(Vec3::new(0.0, 0.0, 1.4 / 1.5), 1e-5));
}

#[test]
fn an_agent_walks_at_its_own_speed_outside_the_slow_radius() {
    let mut app = app();
    let point = spawn_point(&mut app, Vec3::new(0.0, 0.0, 10.0));
    let agent = world(&mut app).spawn((Transform::default(), NavAgent::new().with_speed(2.0)));
    settle(&mut app);
    world(&mut app).insert(NavDestination::to_point(point), agent);

    app.update();

    let velocity = steering(&mut app, agent).velocity;
    assert!(velocity.abs_diff_eq(Vec3::new(0.0, 0.0, 2.0), 1e-5));
}

#[test]
fn a_requested_speed_overrides_the_agents() {
    let mut app = app();
    let point = spawn_point(&mut app, Vec3::new(0.0, 0.0, 10.0));
    let agent = spawn_agent(&mut app, Vec3::ZERO);
    settle(&mut app);
    world(&mut app).insert(NavDestination::to_point(point).with_speed(3.0), agent);

    app.update();

    let velocity = steering(&mut app, agent).velocity;
    assert!(velocity.abs_diff_eq(Vec3::new(0.0, 0.0, 3.0), 1e-5));
}

#[test]
fn passing_a_waypoint_advances_past_it_without_turning_back() {
    let Corner {
        mut app, agent, c, ..
    } = corner();
    world(&mut app)
        .get_component_for_entity_mut::<Transform>(agent)
        .unwrap()
        .translation = Vec3::new(0.0, 0.0, -0.1);
    app.update();
    world(&mut app).insert(NavDestination::to_point(c), agent);
    app.update();
    assert_eq!(
        route(&mut app, agent).unwrap().remaining(),
        &[Vec3::new(0.0, 0.0, 4.0), Vec3::new(4.0, 0.0, 4.0)]
    );

    world(&mut app)
        .get_component_for_entity_mut::<Transform>(agent)
        .unwrap()
        .translation = Vec3::new(0.3, 0.0, 4.6);
    app.update();
    app.update();

    assert_eq!(
        route(&mut app, agent).unwrap().remaining(),
        &[Vec3::new(4.0, 0.0, 4.0)]
    );
    assert!(steering(&mut app, agent).velocity.x > 0.0);
}

#[test]
fn an_agent_that_has_not_planned_has_no_steering() {
    let Corner { mut app, agent, .. } = corner();

    app.update();

    assert!(
        world(&mut app)
            .get_component_for_entity::<NavSteering>(agent)
            .is_none()
    );
}

#[test]
fn an_agent_whose_first_destination_is_unreachable_gets_no_steering() {
    let mut app = app();
    let point = world(&mut app).spawn((
        Transform::from_translation(Vec3::new(0.0, 0.0, 10.0)),
        NavPoint {
            anchor: Name::default(),
            enabled: false,
        },
    ));
    let agent = spawn_agent(&mut app, Vec3::ZERO);
    settle(&mut app);
    world(&mut app).insert(NavDestination::to_point(point), agent);

    app.update();

    assert_eq!(
        events::<NavUnreachable>(&app),
        vec![NavUnreachable {
            agent,
            target: NavTarget::Point(point),
        }]
    );
    assert!(
        world(&mut app)
            .get_component_for_entity::<NavSteering>(agent)
            .is_none()
    );
}
