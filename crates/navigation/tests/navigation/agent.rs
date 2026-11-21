use std::{
    f32::consts::FRAC_PI_2,
    time::{Duration, Instant},
};

use concerto_app::{
    App,
    main_schedule::MainSchedulePlugin,
    plugins::{AssetManagerPlugin, TimePlugin, TransformPlugin},
    schedule_groups::Update,
};
use concerto_ecs::{
    Entity, IntoSystemConfig, Query, World,
    component::scene::SceneEntityRef,
    events::{Event, event_channel::EventChannel, event_cursor::EventCursor},
};
use concerto_foundation::{
    assets::{AssetId, asset_server::AssetServer, handle::AssetHandle},
    transform::Transform,
};
use concerto_name::Name;
use concerto_navigation::{
    NavAgent, NavArrived, NavDestination, NavLink, NavLinks, NavMesh, NavMeshComponent,
    NavMeshData, NavMeshGeometry, NavMeshSettings, NavPoint, NavRoute, NavSteering, NavTarget,
    NavUnreachable, NavigationPlugin, bake, steer_nav_agents,
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
fn an_agent_sent_to_an_anchor_walks_there_and_takes_the_points_facing() {
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
    assert!(position(&mut app, agent).distance(Vec3::new(4.0, 0.0, 4.0)) <= 0.25 + 1e-3);
}

#[test]
fn disabling_a_point_on_the_route_replans_around_it_until_no_way_is_left() {
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

    disable(&mut app, b);

    let remaining = route(&mut app, agent).unwrap().remaining().to_vec();
    assert!(!remaining.contains(&Vec3::new(0.0, 0.0, 4.0)));
    assert_eq!(remaining.last(), Some(&Vec3::new(4.0, 0.0, 4.0)));
    assert!(steering(&mut app, agent).velocity != Vec3::ZERO);

    disable(&mut app, d);

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

fn disable(app: &mut App, point: Entity) {
    world(app)
        .get_component_for_entity_mut::<NavPoint>(point)
        .unwrap()
        .enabled = false;
    app.update();
    app.update();
}

struct Pace {
    agent: NavAgent,
    distance: f32,
    requested: Option<f32>,
    speed: f32,
}

#[test]
fn an_agent_walks_at_the_requested_speed_or_its_own_slowing_inside_the_slow_radius() {
    let paces = [
        Pace {
            agent: NavAgent::new().with_speed(2.0),
            distance: 10.0,
            requested: None,
            speed: 2.0,
        },
        Pace {
            agent: NavAgent::new(),
            distance: 10.0,
            requested: Some(3.0),
            speed: 3.0,
        },
        Pace {
            agent: NavAgent::new(),
            distance: 1.0,
            requested: None,
            speed: 1.4 / 1.5,
        },
    ];
    for pace in paces {
        let mut app = app();
        let point = spawn_point(&mut app, Vec3::new(0.0, 0.0, pace.distance));
        let agent = world(&mut app).spawn((Transform::default(), pace.agent));
        settle(&mut app);
        let destination = NavDestination::to_point(point);
        let destination = match pace.requested {
            Some(speed) => destination.with_speed(speed),
            None => destination,
        };
        world(&mut app).insert(destination, agent);

        app.update();

        let velocity = steering(&mut app, agent).velocity;
        assert!(
            velocity.abs_diff_eq(Vec3::new(0.0, 0.0, pace.speed), 1e-5),
            "expected {}, got {velocity}",
            pace.speed
        );
    }
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

fn position(app: &mut App, agent: Entity) -> Vec3 {
    world(app)
        .get_component_for_entity::<Transform>(agent)
        .unwrap()
        .translation
}

#[test]
fn removing_the_destination_stops_the_agent() {
    let Corner {
        mut app, agent, c, ..
    } = corner();
    world(&mut app).insert(NavDestination::to_point(c), agent);
    for _ in 0..20 {
        app.update();
    }
    assert!(steering(&mut app, agent).velocity != Vec3::ZERO);

    world(&mut app).remove_component::<NavDestination>(agent);

    let facing = steering(&mut app, agent).facing;
    app.update();
    assert_eq!(steering(&mut app, agent).velocity, Vec3::ZERO);
    assert_eq!(steering(&mut app, agent).facing, facing);
    assert!(route(&mut app, agent).is_none());
    let stopped_at = position(&mut app, agent);
    for _ in 0..10 {
        app.update();
    }
    assert_eq!(position(&mut app, agent), stopped_at);
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
fn a_route_follows_the_graph_until_a_mesh_lands_then_its_corners_until_it_goes() {
    let Across {
        mut app,
        agent,
        east,
    } = across();
    world(&mut app).insert(NavDestination::to_point(east), agent);
    for _ in 0..5 {
        app.update();
    }
    let on_the_graph = route(&mut app, agent).unwrap();
    assert_eq!(on_the_graph.planned_on(), None);
    assert_eq!(on_the_graph.remaining(), &[EAST]);

    let Placed { holder, id } = spawn_mesh(&mut app);
    for _ in 0..10 {
        app.update();
    }

    let on_the_mesh = route(&mut app, agent).unwrap();
    assert_eq!(on_the_mesh.planned_on(), Some(id));
    assert_eq!(on_the_mesh.destination(), east);
    assert!(
        goes_around_the_box(on_the_mesh.remaining()),
        "{on_the_mesh:?}"
    );
    assert!(on_the_mesh.remaining().last().unwrap().distance(EAST) < 0.1);

    world(&mut app).remove_component::<NavMeshComponent>(holder);
    app.update();

    let back = route(&mut app, agent).unwrap();
    assert_eq!(back.planned_on(), None);
    assert_eq!(back.remaining().last(), Some(&EAST));
}

#[test]
fn moving_the_destination_mid_route_replans_a_mesh_route_toward_it() {
    let Across {
        mut app,
        agent,
        east,
    } = across();
    spawn_mesh(&mut app);
    world(&mut app).insert(NavDestination::to_point(east), agent);
    for _ in 0..5 {
        app.update();
    }
    let moved = Vec3::new(3.0, 0.0, 3.0);

    world(&mut app)
        .get_component_for_entity_mut::<Transform>(east)
        .unwrap()
        .translation = moved;
    app.update();
    app.update();

    let route = route(&mut app, agent).unwrap();
    assert!(route.planned_on().is_some());
    assert!(
        route.remaining().last().unwrap().distance(moved) < 0.1,
        "{route:?}"
    );
}

#[test]
fn a_mesh_that_failed_to_load_or_has_no_polygons_leaves_agents_on_the_graph() {
    for failed in [true, false] {
        let Across {
            mut app,
            agent,
            east,
        } = across();
        let handle = if failed {
            let server = app.get_resource::<AssetServer>().unwrap().clone();
            server.load::<NavMesh>(AssetId::new())
        } else {
            add_mesh(
                &mut app,
                NavMesh::baked(NavMeshSettings::new(), NavMeshData::default()),
            )
        };
        world(&mut app).spawn(NavMeshComponent { handle });
        world(&mut app).insert(NavDestination::to_point(east), agent);
        let deadline = Instant::now() + Duration::from_secs(10);
        let route = loop {
            app.update();
            if let Some(route) = route(&mut app, agent) {
                break route;
            }
            assert!(has_destination(&mut app, agent), "the agent gave up");
            assert!(Instant::now() < deadline, "the agent never planned");
            std::thread::sleep(Duration::from_millis(1));
        };
        assert_eq!(route.planned_on(), None, "failed load: {failed}");
        assert_eq!(route.remaining(), &[EAST], "failed load: {failed}");
    }
}
