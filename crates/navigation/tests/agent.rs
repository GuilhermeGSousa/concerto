use std::f32::consts::FRAC_PI_2;

use concerto_app::{
    App,
    main_schedule::MainSchedulePlugin,
    plugins::{AssetManagerPlugin, TimePlugin, TransformPlugin},
    schedule_groups::Update,
};
use concerto_ecs::{
    Entity, IntoSystemConfig, Query, ResMut, Resource, World,
    component::scene::SceneEntityRef,
    events::{Event, event_channel::EventChannel, event_cursor::EventCursor},
    signal::{On, lifecycle::Add},
};
use concerto_foundation::{
    assets::{AssetId, asset_server::AssetServer, handle::AssetHandle},
    transform::Transform,
};
use concerto_name::Name;
use concerto_navigation::{
    NavAgent, NavArrived, NavDestination, NavLink, NavLinks, NavMesh, NavMeshComponent,
    NavMeshGeometry, NavMeshSettings, NavPoint, NavRoute, NavSteering, NavTarget, NavUnreachable,
    NavigationPlugin, bake, steer_nav_agents,
};
use glam::{Affine3A, Quat, Vec3};

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
        .register_plugin(AssetManagerPlugin)
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

fn position(app: &mut App, agent: Entity) -> Vec3 {
    world(app)
        .get_component_for_entity::<Transform>(agent)
        .unwrap()
        .translation
}

fn walk_part_way() -> Corner {
    let mut corner = corner();
    world(&mut corner.app).insert(NavDestination::to_point(corner.c), corner.agent);
    for _ in 0..20 {
        corner.app.update();
    }
    assert!(steering(&mut corner.app, corner.agent).velocity != Vec3::ZERO);
    corner
}

fn assert_stopped(app: &mut App, agent: Entity) {
    let facing = steering(app, agent).facing;
    app.update();
    assert_eq!(steering(app, agent).velocity, Vec3::ZERO);
    assert_eq!(steering(app, agent).facing, facing);
    assert!(route(app, agent).is_none());
    let stopped_at = position(app, agent);
    for _ in 0..10 {
        app.update();
    }
    assert_eq!(position(app, agent), stopped_at);
}

#[test]
fn removing_the_destination_stops_the_agent() {
    let Corner { mut app, agent, .. } = walk_part_way();

    world(&mut app).remove_component::<NavDestination>(agent);

    assert_stopped(&mut app, agent);
}

#[test]
fn removing_the_agent_stops_its_steering() {
    let Corner { mut app, agent, .. } = walk_part_way();

    world(&mut app).remove_component::<NavAgent>(agent);

    assert_stopped(&mut app, agent);
}

#[derive(Resource, Default)]
struct Plans {
    count: usize,
}

fn count_plans(_: On<Add<NavRoute>>, mut plans: ResMut<Plans>) {
    plans.count += 1;
}

fn plans(app: &App) -> usize {
    app.get_resource::<Plans>().unwrap().count
}

fn obstacle_mesh() -> NavMesh {
    let mut geometry = NavMeshGeometry::new();
    geometry.add_box(
        Affine3A::from_translation(Vec3::new(0.0, -0.5, 0.0)),
        Vec3::new(5.0, 0.5, 5.0),
    );
    geometry.add_box(
        Affine3A::from_translation(Vec3::new(0.0, 0.5, 0.0)),
        Vec3::splat(0.5),
    );
    let settings = NavMeshSettings::new();
    NavMesh::baked(settings.clone(), bake(&geometry, &settings).unwrap())
}

fn add_mesh(app: &mut App, navmesh: NavMesh) -> AssetHandle<NavMesh> {
    let server = app.get_resource::<AssetServer>().unwrap().clone();
    server.add(navmesh)
}

struct Across {
    app: App,
    agent: Entity,
    east: Entity,
}

const WEST: Vec3 = Vec3::new(-3.0, 0.0, 0.0);
const EAST: Vec3 = Vec3::new(3.0, 0.0, 0.0);

fn across() -> Across {
    let mut app = app();
    app.insert_resource(Plans::default());
    world(&mut app).add_listener(count_plans);
    let west = spawn_point(&mut app, WEST);
    let east = spawn_point(&mut app, EAST);
    link(&mut app, west, &[east]);
    let agent = spawn_agent(&mut app, WEST);
    settle(&mut app);
    Across { app, agent, east }
}

struct Placed {
    holder: Entity,
    id: AssetId,
}

fn spawn_mesh(app: &mut App) -> Placed {
    let handle = add_mesh(app, obstacle_mesh());
    let id = handle.id();
    let holder = world(app).spawn(NavMeshComponent { handle });
    settle(app);
    Placed { holder, id }
}

fn goes_around_the_box(remaining: &[Vec3]) -> bool {
    remaining.iter().any(|corner| corner.z.abs() > 0.5)
}

#[test]
fn with_a_mesh_a_route_follows_its_corners_around_an_obstacle() {
    let Across {
        mut app,
        agent,
        east,
        ..
    } = across();
    let Placed { id, .. } = spawn_mesh(&mut app);
    world(&mut app).insert(NavDestination::to_point(east), agent);

    app.update();

    let route = route(&mut app, agent).unwrap();
    assert_eq!(route.planned_on(), Some(id));
    assert_eq!(route.destination(), east);
    assert!(goes_around_the_box(route.remaining()), "{route:?}");
    assert!(route.remaining().last().unwrap().distance(EAST) < 0.1);
    assert_eq!(
        walk_until_arrived(&mut app, 600),
        Some(NavArrived { agent, point: east })
    );
}

#[test]
fn without_a_mesh_a_route_follows_the_graph() {
    let Across {
        mut app,
        agent,
        east,
        ..
    } = across();
    world(&mut app).insert(NavDestination::to_point(east), agent);

    app.update();

    let route = route(&mut app, agent).unwrap();
    assert_eq!(route.planned_on(), None);
    assert_eq!(route.remaining(), &[EAST]);
}

#[test]
fn a_mesh_landing_mid_route_replans_once() {
    let Across {
        mut app,
        agent,
        east,
        ..
    } = across();
    world(&mut app).insert(NavDestination::to_point(east), agent);
    for _ in 0..5 {
        app.update();
    }
    assert_eq!(plans(&app), 1);

    let Placed { id, .. } = spawn_mesh(&mut app);
    for _ in 0..10 {
        app.update();
    }

    assert_eq!(plans(&app), 2);
    let route = route(&mut app, agent).unwrap();
    assert_eq!(route.planned_on(), Some(id));
    assert!(goes_around_the_box(route.remaining()), "{route:?}");
}

#[test]
fn removing_the_mesh_component_replans_onto_the_graph() {
    let Across {
        mut app,
        agent,
        east,
        ..
    } = across();
    let Placed { holder, .. } = spawn_mesh(&mut app);
    world(&mut app).insert(NavDestination::to_point(east), agent);
    app.update();
    assert!(route(&mut app, agent).unwrap().planned_on().is_some());

    world(&mut app).remove_component::<NavMeshComponent>(holder);
    app.update();

    assert_eq!(plans(&app), 2);
    let route = route(&mut app, agent).unwrap();
    assert_eq!(route.planned_on(), None);
    assert!(!goes_around_the_box(route.remaining()), "{route:?}");
    assert_eq!(route.remaining().last(), Some(&EAST));
}

#[test]
fn a_route_does_not_replan_on_quiet_frames() {
    let Across {
        mut app,
        agent,
        east,
        ..
    } = across();
    spawn_mesh(&mut app);
    world(&mut app).insert(NavDestination::to_point(east), agent);

    for _ in 0..10 {
        app.update();
    }

    assert_eq!(plans(&app), 1);
}

#[test]
fn the_first_component_with_a_loaded_mesh_is_used() {
    let Across {
        mut app,
        agent,
        east,
        ..
    } = across();
    world(&mut app).spawn(NavMeshComponent {
        handle: AssetHandle::weak(AssetId::new()),
    });
    let Placed { id, .. } = spawn_mesh(&mut app);
    world(&mut app).insert(NavDestination::to_point(east), agent);

    app.update();

    assert_eq!(route(&mut app, agent).unwrap().planned_on(), Some(id));
}

#[test]
fn a_disabled_destination_is_unreachable_on_the_mesh() {
    let Across {
        mut app,
        agent,
        east,
        ..
    } = across();
    spawn_mesh(&mut app);
    world(&mut app).insert(
        NavPoint {
            anchor: Name::default(),
            enabled: false,
        },
        east,
    );
    settle(&mut app);
    world(&mut app).insert(NavDestination::to_point(east), agent);

    app.update();

    assert_eq!(
        events::<NavUnreachable>(&app),
        vec![NavUnreachable {
            agent,
            target: NavTarget::Point(east),
        }]
    );
    assert!(route(&mut app, agent).is_none());
}
